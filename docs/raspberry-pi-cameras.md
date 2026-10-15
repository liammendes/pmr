# Self-healing camera services on Raspberry Pi

A camera should spend its day capturing frames, not waiting for you to SSH in and restart a worker. PMR can supervise the capture, streaming and upload processes that make a Raspberry Pi camera useful.

When a worker exits unexpectedly, pmr can restart it with backoff. With startup configured and the process list saved, it can restore the service after the Pi reboots. Each component gets its own name, restart history and logs, making failures easier to find.

## Start with your capture application

Install pmr on 64-bit Linux and put your tested camera application at `/opt/camera/capture.py`, or edit [the example configuration](../examples/camera.ecosystem.json) to point at your executable. The configuration is a supervisor template; it does not include a camera driver or capture implementation.

```sh
pmr start examples/camera.ecosystem.json
pmr logs camera-capture --nostream --lines 30
pmr save
pmr startup systemd
```

Review the startup command that pmr prints, run it as instructed and verify recovery with a controlled reboot. Ensure the service user has the device permissions your camera stack needs.

The example begins with a one-second exponential restart delay, a ten-second minimum uptime and a twenty-restart crash limit. Tune those settings for your device. Repeated early failures eventually leave the process errored so a broken configuration does not retry indefinitely.

## Make failure detectable

A process supervisor sees process exits. A worker that is alive but has stopped producing frames needs an application health check. Have your capture application detect stale frames or a failed camera connection, release the device cleanly and exit with an error so the restart policy can act. Monitor output freshness as well as PID status.

Test camera disconnects, upload outages, storage limits and orderly shutdown on your actual device. PMR does not repair failed hardware, restore lost power or guarantee recovery from every driver hang. Log retention, stable power and an appropriate hardware watchdog remain part of a durable camera deployment.

ARM64 builds are included in the release matrix. Raspberry Pi hardware and camera drivers have not been validated in this release; the published speed benchmarks were run on a MacBook Pro.
