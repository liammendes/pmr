import React from 'react';
import {Video} from '@remotion/media';
import {Img, Sequence, staticFile} from 'remotion';

const captureStyle:React.CSSProperties={position:'absolute',left:100,top:425,width:1720,overflow:'hidden',borderRadius:16,boxShadow:'0 3px 5px #12214118, 0 26px 55px #12214128, 0 70px 90px #15358b12'};
const headingStyle:React.CSSProperties={position:'absolute',left:100,top:197,margin:0,fontSize:86,fontWeight:750,letterSpacing:'-.035em',lineHeight:1.1};
const captionStyle:React.CSSProperties={position:'absolute',left:100,right:100,fontSize:28,lineHeight:'41px',color:'#000000'};

export const AgentUseCase:React.FC<{reducedMotion:boolean}>=({reducedMotion})=><>
  <h1 style={headingStyle}>Supervise your AI agents.</h1>
  <div style={{...captionStyle,top:310}}>Run named agent workers. Follow their inputs, steps and results.</div>
  <div style={captureStyle}>
    {reducedMotion
      ? <Img alt="Real PMR log stream from three scripted agent demo processes" src={staticFile('pmr-agent-logs.png')} style={{display:'block',width:'100%'}}/>
      : <Sequence from={1440} durationInFrames={480} layout="none"><Video src={staticFile('pmr-agent-logs.mp4')} muted style={{display:'block',width:'100%'}}/></Sequence>}
  </div>
  <div style={{...captionStyle,top:948}}><span style={{fontFamily:'monospace'}}>pmr logs</span> · Scripted agent demo, captured from real supervised processes.</div>
</>;

export const WorkerUseCase=()=><>
  <h1 style={headingStyle}>Keep background jobs running.</h1>
  <div style={{...captionStyle,top:310}}>Supervise report, archive and checksum workers from one dashboard.</div>
  <div style={captureStyle}><Img alt="Real PMR dashboard with report, archive and checksum workers online, showing completed jobs and processed records" src={staticFile('pmr-background-workers.png')} style={{display:'block',width:'100%'}}/></div>
  <div style={{...captionStyle,top:948}}>Real local jobs · JSON reports, gzip archives and SHA-256 checks.</div>
</>;
