import React from 'react';
import {AbsoluteFill, Composition, Easing, Img, Still, interpolate, staticFile, useCurrentFrame} from 'remotion';

const palette = {paper:'#faf9f6',ink:'#000000',blue:'#0057ff',track:'#e5e5e5'};
const alignment = {left:100};
const wordmarkShadow='drop-shadow(0 3px 0 #788bb133) drop-shadow(0 16px 16px #34476c1f)';
const iconShadow='drop-shadow(0 3px 4px #2d38551a) drop-shadow(0 32px 36px #3e538629)';
const fonts = `@font-face{font-family:Manrope;src:url('${staticFile('fonts/manrope.woff2')}') format('woff2');font-weight:200 800}*{box-sizing:border-box}`;
const easeOut=Easing.bezier(0.23,1,0.32,1);
const easeMove=Easing.bezier(0.77,0,0.175,1);
const progress=(frame:number,start:number,end:number,easing=easeOut)=>interpolate(frame,[start,end],[0,1],{extrapolateLeft:'clamp',extrapolateRight:'clamp',easing});
const shadow='0 3px 5px #12214118, 0 26px 55px #12214128, 0 70px 90px #15358b12';
const Base:React.FC<{children:React.ReactNode}>=({children})=><AbsoluteFill style={{background:palette.paper,color:palette.ink,fontFamily:'Manrope, sans-serif',WebkitFontSmoothing:'antialiased'}}><style>{fonts}</style>{children}</AbsoluteFill>;
const AuthorLogo:React.FC<{style?:React.CSSProperties}>=({style})=> <Img alt="Liam Mendes" src={staticFile('liam-mendes.svg')} style={{position:'absolute',right:24,bottom:24.019,width:148,height:73.5432,...style}}/>;
const Wordmark:React.FC<{style?:React.CSSProperties}>=({style})=><Img alt="pmr" src={staticFile('pmr-wordmark.svg')} style={{width:1000,height:374,filter:wordmarkShadow,...style}}/>;
const TerminalIcon:React.FC<{style?:React.CSSProperties}>=({style})=><Img alt="Deep blue terminal icon" src={staticFile('pmr-icon.png')} style={{width:420,height:420,filter:iconShadow,...style}}/>;
const Screenshot:React.FC<{camera?:boolean;style?:React.CSSProperties}>=({camera=false,style})=><div style={{width:1720,overflow:'hidden',borderRadius:16,boxShadow:shadow,...style}}><Img alt={camera?'Real pmr camera-sim dashboard after one restart':'Real pmr dashboard with four local demo services'} src={staticFile(camera?'pmr-camera-recovery.jpg':'pmr-dashboard.jpg')} style={{width:'100%',display:'block'}}/></div>;
const Caption:React.FC<{children:React.ReactNode;top:number}>=({children,top})=><div style={{position:'absolute',left:alignment.left,right:100,top,fontSize:28,lineHeight:'41px',color:palette.ink}}>{children}</div>;

const Launch:React.FC<{reducedMotion?:boolean}>=({reducedMotion=false})=>{
  const frame=useCurrentFrame();
  const move=progress(frame,210,246,easeMove);
  const heroOpacity=1-progress(frame,204,228);
  const scene=frame<240?0:frame<600?1:frame<1020?2:3;
  const start=[0,240,600,1020][scene];
  const arrive=progress(frame,start+8,start+38);
  const depart=scene===3?0:progress(frame,[210,566,986,1440][scene],[234,594,1014,1440][scene]);
  const opacity=scene===0?heroOpacity:arrive*(1-depart);
  const y=reducedMotion?0:22*(1-arrive)-12*depart;
  const logoX=alignment.left;
  const logoY=reducedMotion?(frame<225?235:64):235-171*move;
  const logoScale=reducedMotion?(frame<225?1:.21):1-.79*move;
  const logoOpacity=reducedMotion&&frame>=204&&frame<246?Math.abs(frame-225)/21:1;
  return <Base>
    <AuthorLogo/>
    <Wordmark style={{position:'absolute',left:0,top:0,transformOrigin:'top left',transform:`translate(${logoX}px,${logoY}px) scale(${logoScale})`,opacity:logoOpacity}}/>
    {scene===0&&<div style={{opacity:heroOpacity}}>
      <div style={{position:'absolute',left:1137,top:281,transformOrigin:'top left',transform:'rotate(346.14deg)'}}>
        <TerminalIcon style={{display:'block',width:256,height:256,transform:reducedMotion?'none':`perspective(1300px) rotateY(${interpolate(progress(frame,0,90),[0,1],[-8,0])}deg) translateY(${interpolate(progress(frame,0,90),[0,1],[15,0])}px)`}}/>
      </div>
      <h1 style={{position:'absolute',left:alignment.left,top:735,margin:0,fontSize:74,fontWeight:750,letterSpacing:'-.035em',lineHeight:'82px'}}>Keep your processes running.</h1>
      <Caption top={842}>A Rust process manager for developers and coding agents.</Caption>
    </div>}
    {scene===1&&<div style={{opacity,transform:`translateY(${y}px)`}}>
      <h1 style={{position:'absolute',left:100,top:197,margin:0,fontSize:86,fontWeight:750,letterSpacing:'-.035em',lineHeight:1.1}}>Every process. One clear view.</h1>
      <Caption top={310}>Names, logs and resource use. Ready for you and your agents.</Caption>
      <Screenshot style={{position:'absolute',left:alignment.left,top:425,transform:reducedMotion?'none':`perspective(2200px) rotateX(${3*(1-arrive)}deg) scale(${.96+.04*arrive})`,transformOrigin:'center top'}}/>
      <Caption top={948}><span style={{fontFamily:'monospace',color:palette.ink}}>pmr monit</span> · Real dashboard, isolated local demo services.</Caption>
    </div>}
    {scene===2&&<div style={{opacity,transform:`translateY(${y}px)`}}>
      <h1 style={{position:'absolute',left:100,top:197,margin:0,fontSize:86,fontWeight:750,letterSpacing:'-.035em',lineHeight:1.1}}>A faster process list.</h1>
      <Caption top={310}>10 processes · JSON output · 30 samples per manager · median time</Caption>
      <div style={{position:'absolute',left:100,top:420,width:1720}}>
        {[{name:'pmr 1.0.0',time:'14.7 ms',fraction:14.704125/120,color:palette.blue},{name:'PM2 7.0.4',time:'100.1 ms',fraction:100.091417/120,color:'#767d90'}].map((bar,i)=><div key={bar.name} style={{marginBottom:58}}>
          <div style={{display:'flex',justifyContent:'space-between',marginBottom:17,fontSize:34,fontWeight:650,fontVariantNumeric:'tabular-nums'}}><span>{bar.name}</span><span>{bar.time}</span></div>
          <div style={{height:60,background:palette.track,borderRadius:12,overflow:'hidden'}}><div style={{height:'100%',width:`${bar.fraction*100}%`,borderRadius:12,background:bar.color,transformOrigin:'left center',transform:`scaleX(${reducedMotion?1:progress(frame,640,676)})`}}/></div>
        </div>)}
      </div>
      <Caption top={816}>Apple M1 Max · 10 CPU cores · 32 GiB RAM · macOS 27.2 · Node 26.4.0</Caption>
      <Caption top={870}>PM2 / pmr median time: <strong style={{color:palette.ink}}>6.8×</strong> for this command. Application speed varies.</Caption>
      <Caption top={924}>Working desktop, 15 Oct 2026. Method, p95 and raw samples in the benchmark report.</Caption>
    </div>}
    {scene===3&&<div style={{opacity,transform:`translateY(${y}px)`}}>
      <h1 style={{position:'absolute',left:100,top:197,margin:0,fontSize:80,fontWeight:750,letterSpacing:'-.035em',lineHeight:1.1}}>Self-healing camera services.</h1>
      <Caption top={310}>Supervise Raspberry Pi capture and upload workers. Recover from process crashes.</Caption>
      <Screenshot camera style={{position:'absolute',left:alignment.left,top:425,transform:reducedMotion?'none':`scale(${.97+.03*arrive})`}}/>
      <Caption top={942}>Synthetic-frame demo on macOS · one real restart · physical Raspberry Pi hardware untested</Caption>
      <div style={{position:'absolute',left:alignment.left,top:1003,fontSize:22,color:palette.ink}}>Reboot recovery requires startup setup.</div>
    </div>}
  </Base>;
};
const Poster=()=> <Base>
  <AuthorLogo style={{left:32,top:32,right:'auto',bottom:'auto',width:96,height:48}}/>
  <Wordmark style={{position:'absolute',left:'50%',top:85,width:670,height:250.58,transform:'translateX(-50%)'}}/>
  <h1 style={{position:'absolute',top:382,left:'50%',transform:'translateX(-50%)',width:'max-content',fontSize:56,fontWeight:750,letterSpacing:'-.035em',lineHeight:'80px',margin:0}}>Keep your processes running.</h1>
  <div style={{position:'absolute',top:462,left:'calc(50% - 0.5px)',transform:'translateX(-50%)',width:'max-content',fontSize:20,lineHeight:'41px',color:palette.ink}}>A Rust process manager for developers and coding agents.</div>
  <Screenshot style={{position:'absolute',left:alignment.left,top:550,boxShadow:'0 3px 5px #12214118, 0 26px 55px #12214128'}}/>
</Base>;
export const Root=()=> <><Composition id="Launch" component={Launch} width={1920} height={1080} fps={60} durationInFrames={1440}/><Composition id="LaunchReducedMotion" component={Launch} defaultProps={{reducedMotion:true}} width={1920} height={1080} fps={60} durationInFrames={1440}/><Still id="Poster" component={Poster} width={1920} height={1080}/></>;
