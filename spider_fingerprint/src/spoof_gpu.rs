use lazy_static::lazy_static;

/// Mac canvas fingerprint.
pub static CANVAS_FP_MAC: &str = r#"(()=>{const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){let o={r:Math.floor(10*Math.random())-5,g:Math.floor(10*Math.random())-5,b:Math.floor(10*Math.random())-5,a:Math.floor(10*Math.random())-5},r=e.width,n=e.height,a=getImageData.apply(t,[0,0,r,n]);for(let i=0;i<n;i++)for(let f=0;f<r;f++){let l=i*(4*r)+4*f;a.data[l+0]+=o.r,a.data[l+1]+=o.g,a.data[l+2]+=o.b,a.data[l+3]+=o.a}t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,'toBlob',{value:function(){return noisify(this,this.getContext('2d')),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,'toDataURL',{value:function(){return noisify(this,this.getContext('2d')),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,'getImageData',{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}}); })();"#;
/// Windows canvas fingerprint.
pub static CANVAS_FP_WINDOWS: &str = r#"(()=>{const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){let o={r:Math.floor(6*Math.random())-3,g:Math.floor(6*Math.random())-3,b:Math.floor(6*Math.random())-3,a:Math.floor(6*Math.random())-3},r=e.width,n=e.height,a=getImageData.apply(t,[0,0,r,n]);for(let f=0;f<r;f++)for(let i=0;i<n;i++){let l=i*(4*r)+4*f;a.data[l+0]+=o.r,a.data[l+1]+=o.g,a.data[l+2]+=o.b,a.data[l+3]+=o.a}t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,'toBlob',{value:function(){return noisify(this,this.getContext('2d')),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,'toDataURL',{value:function(){return noisify(this,this.getContext('2d')),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,'getImageData',{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}}); })();"#;
/// Linux canvas fingerprint.
pub static CANVAS_FP_LINUX: &str = r#"(()=>{const toBlob=HTMLCanvasElement.prototype.toBlob,toDataURL=HTMLCanvasElement.prototype.toDataURL,getImageData=CanvasRenderingContext2D.prototype.getImageData,noisify=function(e,t){const o={r:Math.floor(10*Math.random())-5,g:Math.floor(10*Math.random())-5,b:Math.floor(10*Math.random())-5,a:Math.floor(10*Math.random())-5},r=e.width,n=e.height,a=t.getImageData(0,0,r,n);for(let i=0;i<r*n*4;i+=4)a.data[i]+=o.r,a.data[i+1]+=o.g,a.data[i+2]+=o.b,a.data[i+3]+=o.a;t.putImageData(a,0,0)};Object.defineProperty(HTMLCanvasElement.prototype,'toBlob',{value:function(){return noisify(this,this.getContext('2d')),toBlob.apply(this,arguments)}}),Object.defineProperty(HTMLCanvasElement.prototype,'toDataURL',{value:function(){return noisify(this,this.getContext('2d')),toDataURL.apply(this,arguments)}}),Object.defineProperty(CanvasRenderingContext2D.prototype,'getImageData',{value:function(){return noisify(this.canvas,this),getImageData.apply(this,arguments)}}); })();"#;

/// Fingerprint JS to spoof.
pub static SPOOF_FINGERPRINT: &str = r###"(()=>{const config={random:{value:()=>Math.random(),item:e=>e[Math.floor(e.length*Math.random())],array:e=>new Int32Array([e[Math.floor(e.length*Math.random())],e[Math.floor(e.length*Math.random())]]),items:(e,t)=>{let r=e.length,a=Array(t),n=Array(r);for(t>r&&(t=r);t--;){let o=Math.floor(Math.random()*r);a[t]=e[o in n?n[o]:o],n[o]=--r in n?n[r]:r}return a}},spoof:{webgl:{buffer:e=>{let t=e.prototype.bufferData;Object.defineProperty(e.prototype,'bufferData',{value:function(){let e=Math.floor(10*Math.random()),r=.1*Math.random()*arguments[1][e];return arguments[1][e]+=r,t.apply(this,arguments)}})},parameter:e=>{Object.defineProperty(e.prototype,'getParameter',{value:function(){let a=new Float32Array([1,8192]);switch(arguments[0]){case 3415:return 0;case 3414:return 24;case 35661:return config.random.items([128,192,256]);case 3386:return config.random.array([8192,16384,32768]);case 36349:case 36347:return config.random.item([4096,8192]);case 34047:case 34921:return config.random.items([2,4,8,16]);case 7937:case 33901:case 33902:return a;case 34930:case 36348:case 35660:return config.random.item([16,32,64]);case 34076:case 34024:case 3379:return config.random.item([16384,32768]);case 3413:case 3412:case 3411:case 3410:case 34852:return config.random.item([2,4,8,16]);default:return config.random.item([0,2,4,8,16,32,64,128,256,512,1024,2048,4096])}}})}}}};config.spoof.webgl.buffer(WebGLRenderingContext);config.spoof.webgl.buffer(WebGL2RenderingContext);config.spoof.webgl.parameter(WebGLRenderingContext);config.spoof.webgl.parameter(WebGL2RenderingContext);const rand={noise:()=>Math.floor(Math.random()+(Math.random()<Math.random()?-1:1)*Math.random()),sign:()=>[-1,-1,-1,-1,-1,-1,1,-1,-1,-1][Math.floor(10*Math.random())]};Object.defineProperty(HTMLElement.prototype,'offsetHeight',{get:function(){let e=Math.floor(this.getBoundingClientRect().height);return e&&1===rand.sign()?e+rand.noise():e}});Object.defineProperty(HTMLElement.prototype,'offsetWidth',{get:function(){let e=Math.floor(this.getBoundingClientRect().width);return e&&1===rand.sign()?e+rand.noise():e}});const ctx={BUFFER:null,getChannelData:e=>{let t=e.prototype.getChannelData;Object.defineProperty(e.prototype,'getChannelData',{value:function(){let d=t.apply(this,arguments);if(ctx.BUFFER!==d){ctx.BUFFER=d;for(let i=0;i<d.length;i+=100){d[Math.floor(Math.random()*i)]+=1e-7*Math.random()}}return d}})},createAnalyser:e=>{let t=e.prototype.__proto__.createAnalyser;Object.defineProperty(e.prototype.__proto__,'createAnalyser',{value:function(){let a=t.apply(this,arguments),r=a.__proto__.getFloatFrequencyData;Object.defineProperty(a.__proto__,'getFloatFrequencyData',{value:function(){let arr=r.apply(this,arguments);for(let i=0;i<arguments[0].length;i+=100){arguments[0][Math.floor(Math.random()*i)]+=.1*Math.random()}return arr}});return a}})} };ctx.getChannelData(AudioBuffer);ctx.createAnalyser(AudioContext);ctx.getChannelData(OfflineAudioContext);ctx.createAnalyser(OfflineAudioContext);window.webkitRTCPeerConnection=void 0;window.RTCPeerConnection=void 0;window.MediaStreamTrack=void 0; })();"###;

/// WebRTC removal and media streams.
pub static SPOOF_RTC: &str = r#"window.webkitRTCPeerConnection=void 0;window.RTCPeerConnection=void 0;window.MediaStreamTrack=void 0;"#;

/// Base fingerprint JS.
pub static BASE_FP_JS: &str = r#"{{CANVAS_FP}}{{SPOOF_FINGERPRINT}}"#;

lazy_static! {
    /// Fingerprint gpu is not enabled for Mac.
    pub static ref FP_JS_MAC: String =  BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_MAC, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1).replace("\n", "");
    /// Fingerprint gpu was enabled on the Mac. The full spoof is not required.
    pub static ref FP_JS_GPU_MAC: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_MAC, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1).replace("\n", "");
    /// Fingerprint gpu is not enabled for Linux.
    pub static ref FP_JS_LINUX: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// Fingerprint gpu was enabled on the Linux. The full spoof is not required.
    pub static ref FP_JS_GPU_LINUX: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
    /// Fingerprint gpu is not enabled for WINDOWS.
    pub static ref FP_JS_WINDOWS: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_WINDOWS, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// Fingerprint gpu was enabled on the WINDOWS. The full spoof is not required.
    pub static ref FP_JS_GPU_WINDOWS: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_WINDOWS, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
}

#[cfg(target_os = "macos")]
lazy_static! {
    /// The gpu is not enabled.
    pub static ref FP_JS: String = FP_JS_MAC.clone();
    /// The gpu was enabled on the machine. The spoof is not required.
    pub static ref FP_JS_GPU: String = FP_JS_GPU_MAC.clone();
}

#[cfg(target_os = "windows")]
lazy_static! {
    /// The gpu is not enabled.
    pub static ref FP_JS: String = FP_JS_WINDOWS.clone();
    /// The gpu was enabled on the machine. The spoof is not required.
    pub static ref FP_JS_GPU: String = FP_JS_GPU_WINDOWS.clone();
}

#[cfg(target_os = "linux")]
lazy_static! {
    /// The gpu is not enabled.
    pub static ref FP_JS: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// The gpu was enabled on the machine. The spoof is not required.
    pub static ref FP_JS_GPU: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
lazy_static! {
    /// The gpu is not enabled.
    pub static ref FP_JS: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", SPOOF_FINGERPRINT, 1) .replace("\n", "");
    /// The gpu was enabled on the machine. The spoof is not required.
    pub static ref FP_JS_GPU: String = BASE_FP_JS.replacen("{{CANVAS_FP}}", CANVAS_FP_LINUX, 1).replacen("{{SPOOF_FINGERPRINT}}", "", 1) .replace("\n", "");
}

/// Build the gpu main spoof script for WGSLLanguageFeatures and canvas.
pub fn build_gpu_spoof_script_wgsl(canvas_format: &str) -> String {
    format!(
        r#"(() =>{{class WGSLanguageFeatures{{constructor(){{this.size=4}}}}class GPU{{constructor(){{this.wgslLanguageFeatures=new WGSLanguageFeatures()}}requestAdapter(){{return Promise.resolve({{requestDevice:()=>Promise.resolve({{}})}})}}getPreferredCanvasFormat(){{return'{canvas_format}'}}}}const _gpu=new GPU(),_g=()=>_gpu;Object.defineProperty(_g,'toString',{{value:()=>`function get gpu() {{ [native code] }}`,configurable:!0}});Object.defineProperty(Navigator.prototype,'gpu',{{get:_g,configurable:!0,enumerable:!1}});if(typeof WorkerNavigator!=='undefined'){{Object.defineProperty(WorkerNavigator.prototype,'gpu',{{get:_g,configurable:!0,enumerable:!1}})}}}})();"#
    )
}
