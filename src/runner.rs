use crate::history::{CommandSpec, Request, Store};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::{watch, Notify};

pub struct Capacity {
    max: AtomicUsize,
    running: AtomicUsize,
    pub changed: Notify,
}
pub struct Permit(Arc<Capacity>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_one();
    }
}
impl Capacity {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self {
            max: AtomicUsize::new(max),
            running: AtomicUsize::new(0),
            changed: Notify::new(),
        })
    }
    pub fn acquire(self: &Arc<Self>) -> Option<Permit> {
        let mut n = self.running.load(Ordering::SeqCst);
        loop {
            if n >= self.max.load(Ordering::SeqCst) {
                return None;
            }
            match self
                .running
                .compare_exchange_weak(n, n + 1, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Some(Permit(Arc::clone(self))),
                Err(current) => n = current,
            }
        }
    }
}
pub struct Runner {
    pub store: Arc<Store>,
    pub capacity: Arc<Capacity>,
    stopping: AtomicBool,
    active: Mutex<HashMap<String, watch::Sender<bool>>>,
    dispatch: tokio::sync::Mutex<()>,
}
impl Runner {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        let store = Store::open(path)?;
        store.recover()?;
        let max = store.settings()?.max_running;
        Ok(Arc::new(Self {
            store,
            capacity: Capacity::new(max),
            stopping: AtomicBool::new(false),
            dispatch: tokio::sync::Mutex::new(()),
            active: Mutex::new(HashMap::new()),
        }))
    }
    pub fn start(self: &Arc<Self>) {
        let runner = Arc::clone(self);
        tokio::spawn(async move {
            let mut last_prune = Instant::now();
            while !runner.stopping.load(Ordering::SeqCst) {
                let mut queue_empty = false;
                // At most 64 launches per scheduling pass, even with a high configured limit.
                for _ in 0..64 {
                    let dispatch = runner.dispatch.lock().await;
                    if runner.stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let Some(permit) = runner.capacity.acquire() else {
                        break;
                    };
                    let next = runner.store.call(Store::claim).await;
                    let (seq, spec) = match next {
                        Ok(Some(next)) => next,
                        Ok(None) => {
                            queue_empty = true;
                            break;
                        }
                        Err(e) => {
                            eprintln!("command queue: {e}");
                            break;
                        }
                    };
                    let (cancel, rx) = watch::channel(false);
                    runner
                        .active
                        .lock()
                        .unwrap()
                        .insert(spec.id.clone(), cancel.clone());
                    if runner.stopping.load(Ordering::SeqCst) {
                        let _ = cancel.send(true);
                    }
                    drop(dispatch);
                    let task = Arc::clone(&runner);
                    tokio::spawn(async move {
                        let id = spec.id.clone();
                        if let Err(e) = task.execute(seq, spec, rx).await {
                            eprintln!("command {id}: {e}");
                            let _ = task
                                .store
                                .call(move |s| s.finish(seq, "failed", None))
                                .await;
                        }
                        task.active.lock().unwrap().remove(&id);
                        drop(permit);
                    });
                }
                if last_prune.elapsed() >= Duration::from_secs(60) {
                    if let Err(e) = runner.store.call(|s| s.prune(100)).await {
                        eprintln!("retention: {e}");
                    }
                    last_prune = Instant::now();
                }
                if queue_empty {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                tokio::select! {
                    _=runner.capacity.changed.notified()=>{},
                    _=tokio::time::sleep(Duration::from_millis(100))=>{},
                }
            }
        });
    }
    async fn execute(
        &self,
        seq: i64,
        spec: CommandSpec,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<()> {
        if *cancel.borrow() {
            let status = if self.stopping.load(Ordering::SeqCst) {
                "interrupted"
            } else {
                "cancelled"
            };
            return self.store.call(move |s| s.finish(seq, status, None)).await;
        }
        let cap = self.store.call(Store::settings).await?.output_bytes;
        let mut command = Command::new(&spec.program);
        command
            .args(&spec.args)
            .current_dir(&spec.cwd)
            .env_clear()
            .envs(&spec.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        {
            command.process_group(0);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let message = e.to_string().into_bytes();
                self.store
                    .call(move |s| {
                        s.append(seq, "err", &message[..message.len().min(4096)], cap)?;
                        s.finish(seq, "failed", None)
                    })
                    .await?;
                return Ok(());
            }
        };
        let pid = child.id().unwrap_or(0);
        let pid_result = self.store.call(move |s| s.set_pid(seq, pid)).await;
        if let Err(error) = pid_result {
            kill_group(&mut child, pid);
            let _ = child.wait().await;
            return Err(error);
        }
        let mut out = tokio::spawn(capture(
            Arc::clone(&self.store),
            seq,
            "out",
            cap,
            child.stdout.take().unwrap(),
        ));
        let mut err = tokio::spawn(capture(
            Arc::clone(&self.store),
            seq,
            "err",
            cap,
            child.stderr.take().unwrap(),
        ));
        let out_abort = out.abort_handle();
        let err_abort = err.abort_handle();
        let drained = async {
            tokio::try_join!(async { (&mut out).await? }, async { (&mut err).await? })?;
            Ok::<(), anyhow::Error>(())
        };
        tokio::pin!(drained);
        let mut capture_ok = true;
        let mut already_drained = false;
        let (cancelled, status) = tokio::select! {
            result=child.wait()=>(false,result?),
            _=cancel.changed()=>{
                kill_group(&mut child,pid);
                (true,child.wait().await?)
            }
            result=&mut drained=>{
                already_drained=true;
                if let Err(error)=result {
                    eprintln!("command output: {error}");
                    capture_ok=false;
                    kill_group(&mut child,pid);
                }
                tokio::select! {
                    result=child.wait()=>(false,result?),
                    _=cancel.changed()=>{
                        kill_group(&mut child,pid);
                        (true,child.wait().await?)
                    }
                }
            }
        };
        if !already_drained {
            capture_ok = matches!(
                tokio::time::timeout(Duration::from_secs(5), &mut drained).await,
                Ok(Ok(()))
            );
        }
        // Descendants must not outlive a one-shot command or retain its pipes.
        kill_group(&mut child, pid);
        if !capture_ok {
            out_abort.abort();
            err_abort.abort();
        }
        let status_name = if cancelled {
            if self.stopping.load(Ordering::SeqCst) {
                "interrupted"
            } else {
                "cancelled"
            }
        } else if !capture_ok {
            "output_incomplete"
        } else if status.success() {
            "succeeded"
        } else {
            "failed"
        };
        let code = status.code();
        self.store
            .call(move |s| s.finish(seq, status_name, code))
            .await
    }
    pub async fn handle(&self, request: Request) -> Result<Value> {
        match request {
            Request::Submit(spec) => {
                if self.stopping.load(Ordering::SeqCst) {
                    return Err(anyhow!("daemon is shutting down"));
                }
                let row = self.store.call(move |s| s.submit(spec, "run")).await?;
                self.capacity.changed.notify_one();
                Ok(serde_json::to_value(row)?)
            }
            Request::List(q) => self.store.call(move |s| s.query(q)).await,
            Request::Get { id } => {
                self.store
                    .call(move |s| Ok(serde_json::to_value(s.get(&id)?)?))
                    .await
            }
            Request::Output { id, lines, stream } => {
                self.store
                    .call(move |s| s.output_filtered(&id, lines, stream.as_deref()))
                    .await
            }
            Request::Prune { batch } => self.store.call(move |s| s.prune(batch)).await,
            Request::Configure(patch) => {
                let _dispatch = self.dispatch.lock().await;
                let settings = self.store.call(move |s| s.configure(patch)).await?;
                self.capacity
                    .max
                    .store(settings.max_running, Ordering::SeqCst);
                self.capacity.changed.notify_one();
                Ok(
                    json!({"settings":settings,"running":self.capacity.running.load(Ordering::SeqCst),
                    "active_commands":self.active.lock().unwrap().len()}),
                )
            }
            Request::Cancel { id } => {
                let _dispatch = self.dispatch.lock().await;
                let target = id.clone();
                let queued = self.store.call(move |s| s.cancel_queued(&target)).await?;
                if !queued {
                    if let Some(sender) = self.active.lock().unwrap().get(&id) {
                        let _ = sender.send(true);
                    } else {
                        return Err(anyhow!("command is not queued or running: {id}"));
                    }
                }
                Ok(json!({"id":id,"cancel_requested":true}))
            }
        }
    }
    pub fn begin_shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.capacity.changed.notify_one();
    }
    pub async fn shutdown(&self) {
        self.begin_shutdown();
        let _dispatch = self.dispatch.lock().await;
        for sender in self.active.lock().unwrap().values() {
            let _ = sender.send(true);
        }
        for _ in 0..100 {
            if self.active.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
async fn capture(
    store: Arc<Store>,
    seq: i64,
    stream: &'static str,
    cap: u64,
    mut reader: impl AsyncRead + Unpin,
) -> Result<()> {
    let mut bytes = [0u8; 4096];
    loop {
        let n = reader.read(&mut bytes).await?;
        if n == 0 {
            break;
        }
        let data = bytes[..n].to_vec();
        store
            .call(move |s| s.append(seq, stream, &data, cap))
            .await?;
    }
    Ok(())
}

fn kill_group(child: &mut tokio::process::Child, pid: u32) {
    #[cfg(unix)]
    if pid > 0 {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    let _ = child.start_kill();
}
