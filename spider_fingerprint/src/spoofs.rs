use crate::builder::AgentOs;
use rand::Rng;

// use https://github.com/spider-rs/headless-browser for ideal default settings.
pub const HIDE_CHROME: &str = "window.chrome={runtime:{}};['log','warn','error','info','debug','table'].forEach((method)=>{console[method]=()=>{}});";
pub const DISABLE_DIALOGS: &str  = "(()=>{const a=window.alert.toString(),c=window.confirm.toString(),p=window.prompt.toString();window.alert=function alert(){};Object.defineProperty(window.alert,'toString',{value:()=>a,configurable:true});window.confirm=function confirm(){return true};Object.defineProperty(window.confirm,'toString',{value:()=>c,configurable:true});window.prompt=function prompt(){return ''};Object.defineProperty(window.prompt,'toString',{value:()=>p,configurable:true});})();";
pub const HIDE_WEBGL: &str = r#"(()=>{const o=WebGLRenderingContext.prototype.getParameter;function h(p){if(p===37445)return'Google Inc. (NVIDIA)';if(p===37446)return'ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Direct3D11 vs_5_0 ps_5_0, D3D11-27.21.14.5671)';return o.call(this,p);}WebGLRenderingContext.prototype.getParameter=h;const w=O=>function(u,...a){const s=`(()=>{const o=WebGLRenderingContext.prototype.getParameter;function h(p){if(p===37445)return'Google Inc. (NVIDIA)';if(p===37446)return'ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Direct3D11 vs_5_0 ps_5_0, D3D11-27.21.14.5671)';return o.call(this,p);}WebGLRenderingContext.prototype.getParameter=h})();importScripts("${u}");`;return new O(URL.createObjectURL(new Blob([s],{type:"application/javascript"})),...a)};window.Worker=w(window.Worker);window.SharedWorker=w(window.SharedWorker);})();"#;
pub const HIDE_WEBGL_MAC: &str = r#"(()=>{const o=WebGLRenderingContext.prototype.getParameter;function h(p){if(p===37445)return'Google Inc. (Apple)';if(p===37446)return'ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Max, Unspecified Version)';return o.call(this,p);}WebGLRenderingContext.prototype.getParameter=h;const w=O=>function(u,...a){const s=`(()=>{const o=WebGLRenderingContext.prototype.getParameter;function h(p){if(p===37445)return'Google Inc. (Apple)';if(p===37446)return'ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Max, Unspecified Version)';return o.call(this,p);}WebGLRenderingContext.prototype.getParameter=h})();importScripts("${u}");`;return new O(URL.createObjectURL(new Blob([s],{type:"application/javascript"})),...a)};window.Worker=w(window.Worker);window.SharedWorker=w(window.SharedWorker);})();"#;
pub const GPU_SPOOF_SCRIPT: &str = r#"(() =>{class WGSLanguageFeatures{constructor(){this.size=4}}class GPU{constructor(){this.wgslLanguageFeatures=new WGSLanguageFeatures()}requestAdapter(){return Promise.resolve({requestDevice:()=>Promise.resolve({})})}getPreferredCanvasFormat(){return'rgba8unorm'}}const _gpu=new GPU(),_g=()=>_gpu;Object.defineProperty(_g,'toString',{value:()=>`function get gpu() { [native code] }`,configurable:true});Object.defineProperty(Navigator.prototype,'gpu',{get:_g,configurable:true,enumerable:false});if(typeof WorkerNavigator!=='undefined'){Object.defineProperty(WorkerNavigator.prototype,'gpu',{get:_g,configurable:true,enumerable:false})}})();"#;
pub const GPU_SPOOF_SCRIPT_MAC: &str = r#"(() =>{class WGSLanguageFeatures{constructor(){this.size=4}}class GPU{constructor(){this.wgslLanguageFeatures=new WGSLanguageFeatures()}requestAdapter(){return Promise.resolve({requestDevice:()=>Promise.resolve({})})}getPreferredCanvasFormat(){return'bgra8unorm'}}const _gpu=new GPU(),_g=()=>_gpu;Object.defineProperty(_g,'toString',{value:()=>`function get gpu() { [native code] }`,configurable:true});Object.defineProperty(Navigator.prototype,'gpu',{get:_g,configurable:true,enumerable:false});if(typeof WorkerNavigator!=='undefined'){Object.defineProperty(WorkerNavigator.prototype,'gpu',{get:_g,configurable:true,enumerable:false})}})();"#;

pub const GPU_REQUEST_ADAPTER: &str = r#"(()=>{const def=(o,m)=>Object.defineProperties(o,Object.fromEntries(Object.entries(m).map(([k,v])=>[k,{value:v,enumerable:true,configurable:true}]))),orig=navigator.gpu.requestAdapter.bind(navigator.gpu),I={vendor:'Google Inc. (NVIDIA)',architecture:'',device:'',description:''},M={maxTextureDimension1D:16384,maxTextureDimension2D:16384,maxTextureDimension3D:2048,maxTextureArrayLayers:2048,maxBindGroups:4,maxBindGroupsPlusVertexBuffers:32,maxBindingsPerBindGroup:1000,maxDynamicUniformBuffersPerPipelineLayout:12,maxDynamicStorageBuffersPerPipelineLayout:16,maxSampledTexturesPerShaderStage:32,maxSamplersPerShaderStage:32,maxStorageBuffersPerShaderStage:12,maxStorageTexturesPerShaderStage:12,maxUniformBuffersPerShaderStage:16,maxUniformBufferBindingSize:131072,maxStorageBufferBindingSize:1073741824,minUniformBufferOffsetAlignment:256,minStorageBufferOffsetAlignment:256,maxVertexBuffers:16,maxBufferSize:4294967296,maxVertexAttributes:16,maxVertexBufferArrayStride:2048,maxInterStageShaderVariables:32,maxColorAttachments:8,maxColorAttachmentBytesPerSample:256,maxComputeWorkgroupStorageSize:65536,maxComputeInvocationsPerWorkgroup:2048,maxComputeWorkgroupSizeX:256,maxComputeWorkgroupSizeY:256,maxComputeWorkgroupSizeZ:64,maxComputeWorkgroupsPerDimension:131072};navigator.gpu.requestAdapter=async opts=>{const a=await orig(opts),lim=a.limits;def(a.info,I);for(const k of Object.getOwnPropertyNames(lim))if(!(k in M))delete lim[k];def(lim,M);return a};})();"#;
pub const GPU_REQUEST_ADAPTER_MAC: &str = r#"(()=>{const def=(o,m)=>Object.defineProperties(o,Object.fromEntries(Object.entries(m).map(([k,v])=>[k,{value:v,enumerable:true,configurable:true}]))),orig=navigator.gpu.requestAdapter.bind(navigator.gpu),M={maxTextureDimension1D:16384,maxTextureDimension2D:16384,maxTextureDimension3D:2048,maxTextureArrayLayers:2048,maxBindGroups:4,maxBindGroupsPlusVertexBuffers:24,maxBindingsPerBindGroup:1000,maxDynamicUniformBuffersPerPipelineLayout:10,maxDynamicStorageBuffersPerPipelineLayout:8,maxSampledTexturesPerShaderStage:16,maxSamplersPerShaderStage:16,maxStorageBuffersPerShaderStage:10,maxStorageTexturesPerShaderStage:8,maxUniformBuffersPerShaderStage:12,maxUniformBufferBindingSize:65536,maxStorageBufferBindingSize:4294967292,minUniformBufferOffsetAlignment:256,minStorageBufferOffsetAlignment:256,maxVertexBuffers:8,maxBufferSize:4294967296,maxVertexAttributes:30,maxVertexBufferArrayStride:2048,maxInterStageShaderVariables:28,maxColorAttachments:8,maxColorAttachmentBytesPerSample:128,maxComputeWorkgroupStorageSize:32768,maxComputeInvocationsPerWorkgroup:1024,maxComputeWorkgroupSizeX:1024,maxComputeWorkgroupSizeY:1024,maxComputeWorkgroupSizeZ:64,maxComputeWorkgroupsPerDimension:65535};navigator.gpu.requestAdapter=async opts=>{const a=await orig(opts),lim=a.limits;def(a.info,{vendor:'apple',architecture:'metal-3',device:'',description:''});for(const k of Object.getOwnPropertyNames(lim))if(!(k in M))delete lim[k];def(lim,M);return a};})();"#;
/// Hide the webdriver from being enabled. You should not need this if you use the cli args to launch chrome - disabled-features=AutomationEnabled
pub const HIDE_WEBDRIVER: &str = r#"(()=>{const r=Function.prototype.toString,g=()=>false;Function.prototype.toString=function(){return this===g?'function get webdriver() { [native code] }':r.call(this)};Object.defineProperty(Navigator.prototype,'webdriver',{get:g,enumerable:false,configurable:true})})();"#;

/// Navigator include pdfViewerEnabled.
pub const NAVIGATOR_SCRIPT: &str = r#"(()=>{const nativeGet=new Function("return true");Object.defineProperty(nativeGet,'toString',{value:()=>"function get pdfViewerEnabled() { [native code] }"});Object.defineProperty(Navigator.prototype,"pdfViewerEnabled",{get:nativeGet,configurable:!0});})();"#;
/// Plugin extension.
pub const PLUGIN_AND_MIMETYPE_SPOOF: &str = r#"(()=>{const m=[{type:'application/pdf',suffixes:'pdf',description:'Portable Document Format'},{type:'text/pdf',suffixes:'pdf',description:'Portable Document Format'}],names=['PDF Viewer','Chrome PDF Viewer','Chromium PDF Viewer','Microsoft Edge PDF Viewer','WebKit built-in PDF'],plugins=[],mimes=[(()=>{const mt={__proto__:MimeType.prototype};Object.defineProperties(mt,{type:{value:m[0].type,configurable:true},suffixes:{value:m[0].suffixes,configurable:true},description:{value:m[0].description,configurable:true}});return mt})(),(()=>{const mt={__proto__:MimeType.prototype};Object.defineProperties(mt,{type:{value:m[1].type,configurable:true},suffixes:{value:m[1].suffixes,configurable:true},description:{value:m[1].description,configurable:true}});return mt})()];names.forEach(name=>{const pl={__proto__:Plugin.prototype,length:2,name,filename:'internal-pdf-viewer',description:'Portable Document Format'};Object.defineProperty(mimes[0],'enabledPlugin',{value:pl,configurable:true});Object.defineProperty(mimes[1],'enabledPlugin',{value:pl,configurable:true});pl[0]={};pl[1]={};plugins.push(pl)});Object.defineProperty(PluginArray.prototype,'item',{value:function(i){return this[i]||null},configurable:true});Object.defineProperty(PluginArray.prototype,'namedItem',{value:function(n){return this[n]||null},configurable:true});Object.defineProperty(MimeTypeArray.prototype,'item',{value:function(i){return this[i]||null},configurable:true});Object.defineProperty(MimeTypeArray.prototype,'namedItem',{value:function(n){return this[n]||null},configurable:true});const pa=Object.create(PluginArray.prototype);plugins.forEach((p,i)=>{Object.defineProperty(pa,i,{value:p,configurable:true,enumerable:true});Object.defineProperty(pa,p.name,{value:p,configurable:true,enumerable:false})});Object.defineProperty(pa,'length',{value:plugins.length,configurable:true});Object.defineProperty(pa,'toJSON',{value:()=>{const o={};for(let i=0;i<plugins.length;i++)o[i]={0:{},1:{}};return o},configurable:true});const ma=Object.create(MimeTypeArray.prototype);mimes.forEach((mt,i)=>{Object.defineProperty(ma,i,{value:mt,configurable:true,enumerable:false});Object.defineProperty(ma,mt.type,{value:mt,configurable:true,enumerable:false})});Object.defineProperty(ma,'length',{value:mimes.length,configurable:true});function g(v,n){const f=()=>v;Object.defineProperty(f,'toString',{value:()=>`function get ${n}() { [native code] }`,configurable:true});return f}Object.defineProperties(Navigator.prototype,{plugins:{get:g(pa,'plugins'),configurable:true,enumerable:false},mimeTypes:{get:g(ma,'mimeTypes'),configurable:true,enumerable:false}})})();"#;
/// Spoof the notifications enabled prompt.
pub const SPOOF_NOTIFICATIONS: &str = r#"(()=>{const a=new Function('return "prompt"');Object.defineProperty(a,'toString',{value:()=>`function get permission() { [native code] }`});Object.defineProperty(Notification,'permission',{get:a,configurable:true});const b=new Function("return function(e){if(e&&e.name==='notifications'){return Promise.resolve(Object.setPrototypeOf({state:'prompt',onchange:null},PermissionStatus.prototype))}return this.__nativeQuery__.apply(this,arguments)}")();Object.defineProperty(b,"toString",{value:()=>`function query() { [native code] }`});navigator.permissions.__nativeQuery__=navigator.permissions.query.bind(navigator.permissions);navigator.permissions.query=b})();"#;
/// Spoof the permissions granted by default.
pub const SPOOF_PERMISSIONS_QUERY: &str = r#"(()=>{const map={accelerometer:"granted","background-fetch":"granted","background-sync":"granted",gyroscope:"granted",magnetometer:"granted","screen-wake-lock":"granted",camera:"prompt","display-capture":"prompt",geolocation:"prompt",microphone:"prompt",midi:"prompt",notifications:"prompt","persistent-storage":"prompt"};const native=navigator.permissions.query.bind(navigator.permissions);Object.defineProperty(navigator.permissions,"query",{value:function(p){if(p&&p.name&&map.hasOwnProperty(p.name)){return Promise.resolve(Object.setPrototypeOf({state:map[p.name],onchange:null},PermissionStatus.prototype))}return native(p)},configurable:true});const g=new Function('return "prompt"');Object.defineProperty(g,"toString",{value:()=>`function get permission() { [native code] }`});Object.defineProperty(Notification,"permission",{get:g,configurable:true})})();"#;

/// Shallow hide-permission spoof. (use SPOOF_PERMISSIONS_QUERY instead.)
pub const HIDE_PERMISSIONS: &str = "(()=>{const originalQuery=window.navigator.permissions.query;window.navigator.permissions.__proto__.query=parameters=>{ return parameters.name === 'notifications' ? Promise.resolve({ state: Notification.permission }) : originalQuery(parameters) }; })();";

/// Spoof Media labels. This will update fake media labels with real ones.
pub fn spoof_media_labels_script(agent_os: AgentOs) -> String {
    let camera_label = match agent_os {
        AgentOs::Mac => "FaceTime HD Camera",
        AgentOs::Windows => "Integrated Webcam",
        AgentOs::Linux => "Integrated Camera",
        AgentOs::Android => "Front Camera",
    };

    format!(
        r#"(()=>{{const e=navigator.mediaDevices.enumerateDevices.bind(navigator.mediaDevices);navigator.mediaDevices.enumerateDevices=()=>e().then(d=>d.map(v=>{{let l=v.label;if(typeof l==="string"){{if(l.startsWith("Fake "))l=l.replace(/^Fake\s+/i,"");if(v.kind==="videoinput"&&/^fake(_device)?/i.test(l))l="{label}";const g=new Function(`return "${{l}}"`);Object.defineProperty(g,"toString",{{value:()=>`function get label() {{ [native code] }}`}});Object.defineProperty(v,"label",{{get:g,configurable:true}})}}return v}}))}})();"#,
        label = camera_label
    )
}

/// Spoof the screen dimensions.
pub fn spoof_screen_script(
    screen_width: u32,
    screen_height: u32,
    device_pixel_ratio: f64,
    emulating_mobile: bool,
    agent_os: AgentOs,
) -> String {
    spoof_screen_script_rng(
        screen_width,
        screen_height,
        device_pixel_ratio,
        emulating_mobile,
        &mut rand::rng(),
        agent_os,
    )
}

/// Spoof the screen dimensions.
pub fn spoof_screen_script_rng<R: Rng>(
    screen_width: u32,
    screen_height: u32,
    device_pixel_ratio: f64,
    emulating_mobile: bool,
    rng: &mut R,
    agent_os: AgentOs,
) -> String {
    // inner size is ~75-90% of screen width/height
    let inner_width =
        rng.random_range((screen_width as f32 * 0.70) as u32..=screen_width.min(1920));
    let inner_height = rng.random_range((screen_height as f32 * 0.75) as u32..=screen_height);

    // outer is typically 60-100px more in height, 0-40 in width
    let outer_height = inner_height + rng.random_range(70..=90); // Chrome on macOS often adds ~75
    let outer_width = inner_width + rng.random_range(0..=20); // Scrollbar / border variance

    let avail_height = screen_height - rng.random_range(30..=60); // menu bar or dock
    let avail_width = screen_width; // leave full unless simulating vertical sidebar

    let (screen_x, screen_y) = if emulating_mobile {
        // Mobile browsers are always full screen — no offset
        (0, 0)
    } else {
        // Desktop: allow offscreen placement (multi-monitor or dragging)
        (
            rng.random_range(-200..=screen_width as i32 + 200),
            rng.random_range(-100..=screen_height as i32 + 100),
        )
    };

    let simulate_hdr = screen_width >= 2560 && device_pixel_ratio >= 2.0;
    let high_end_display = simulate_hdr && matches!(agent_os, AgentOs::Mac | AgentOs::Windows);

    let color_depth = match agent_os {
        AgentOs::Mac | AgentOs::Windows | AgentOs::Linux => {
            if simulate_hdr || high_end_display {
                30
            } else {
                24
            }
        }
        AgentOs::Android => 24,
    };

    format!(
        "(()=>{{const iw=new Function('return {iw}'),ih=new Function('return {ih}'),ow=new Function('return {ow}'),oh=new Function('return {oh}'),sw=new Function('return {sw}'),sh=new Function('return {sh}'),aw=new Function('return {aw}'),ah=new Function('return {ah}'),sx=new Function('return {sx}'),sy=new Function('return {sy}'),cd=new Function('return {cd}'),pd=new Function('return {cd}'),dpr=new Function('return {dpr}');\
        [iw,ih,ow,oh,sw,sh,aw,ah,sx,sy,cd,pd,dpr].forEach((f,i)=>Object.defineProperty(f,'toString',{{value:()=>`function get ${{['innerWidth','innerHeight','outerWidth','outerHeight','width','height','availWidth','availHeight','screenX','screenY','colorDepth','pixelDepth','devicePixelRatio'][i]}}() {{ [native code] }}`}}));\
        Object.defineProperty(window,'innerWidth',{{get:iw,configurable:!0}});\
        Object.defineProperty(window,'innerHeight',{{get:ih,configurable:!0}});\
        Object.defineProperty(window,'outerWidth',{{get:ow,configurable:!0}});\
        Object.defineProperty(window,'outerHeight',{{get:oh,configurable:!0}});\
        Object.defineProperty(window,'screenX',{{get:sx,configurable:!0}});\
        Object.defineProperty(window,'screenY',{{get:sy,configurable:!0}});\
        Object.defineProperty(window,'devicePixelRatio',{{get:dpr,configurable:!0}});\
        Object.defineProperty(Screen.prototype,'width',{{get:sw,configurable:!0}});\
        Object.defineProperty(Screen.prototype,'height',{{get:sh,configurable:!0}});\
        Object.defineProperty(Screen.prototype,'availWidth',{{get:aw,configurable:!0}});\
        Object.defineProperty(Screen.prototype,'availHeight',{{get:ah,configurable:!0}});\
        Object.defineProperty(Screen.prototype,'colorDepth',{{get:cd,configurable:!0}});\
        Object.defineProperty(Screen.prototype,'pixelDepth',{{get:pd,configurable:!0}});\
        }})();",
        iw = inner_width,
        ih = inner_height,
        ow = outer_width,
        oh = outer_height,
        sw = screen_width,
        sh = screen_height,
        aw = avail_width,
        ah = avail_height,
        sx = screen_x,
        sy = screen_y,
        cd = color_depth,
        dpr = device_pixel_ratio
    )
}

/// Resolve the DRP
pub fn resolve_dpr(
    emulating_mobile: bool,
    device_scale_factor: Option<f64>,
    platform: AgentOs,
) -> f64 {
    device_scale_factor.unwrap_or_else(|| {
        if emulating_mobile {
            2.0
        } else {
            match platform {
                AgentOs::Mac => 2.0,
                AgentOs::Linux => 1.0,
                AgentOs::Windows => 1.0,
                AgentOs::Android => 2.0, // can be 3.0+ on some phones, but 2.0 is safe default
            }
        }
    })
}

/// Spoof whether this is a touch screen or not.
pub fn spoof_touch_script(has_touch: bool) -> &'static str {
    if has_touch {
        r#"(()=>{const mtp=new Function('return 1');Object.defineProperty(mtp,'toString',{value:()=>`function get maxTouchPoints() { [native code] }`});Object.defineProperty(Navigator.prototype,'maxTouchPoints',{get:mtp,configurable:true});Object.defineProperty(Navigator.prototype,'msMaxTouchPoints',{get:mtp,configurable:true});try{window.TouchEvent=window.TouchEvent||function(){};Object.defineProperty(window,'ontouchstart',{get:()=>null,configurable:true});Object.defineProperty(document,'ontouchstart',{get:()=>null,configurable:true});}catch{}})();"#
    } else {
        r#"(()=>{const mtp=new Function('return 0');Object.defineProperty(mtp,'toString',{value:()=>`function get maxTouchPoints() { [native code] }`});Object.defineProperty(Navigator.prototype,'maxTouchPoints',{get:mtp,configurable:true});Object.defineProperty(Navigator.prototype,'msMaxTouchPoints',{get:mtp,configurable:true});try{Object.defineProperty(window,'TouchEvent',{get:()=>{throw new ReferenceError('TouchEvent is not defined')}, configurable:true});Object.defineProperty(window,'ontouchstart',{get:()=>undefined, configurable:true});Object.defineProperty(document,'ontouchstart',{get:()=>undefined, configurable:true});}catch{}})();"#
    }
}

/// Spoof the media codecs scripts.
pub fn spoof_media_codecs_script() -> &'static str {
    r#"(()=>{try{const a={"audio/ogg":"probably","audio/mp4":"probably","audio/webm":"probably","audio/wav":"probably"},v={"video/webm":"probably","video/mp4":"probably","video/ogg":"probably"};const c={...a,...v},f=s=>{const[t]=s.split(";");if(t==="video/mp4"&&s.includes("avc1.42E01E"))return{name:t,state:"probably"};const e=Object.entries(c).find(([k])=>k===t);return e?{name:e[0],state:e[1]}:undefined};const h=HTMLMediaElement.prototype,p=h.canPlayType;Object.defineProperty(h,"canPlayType",{value:function(t){if(!t)return p.apply(this,arguments);const r=f(t);return r?r.state:p.apply(this,arguments)},configurable:!0});}catch(e){console.warn(e);}})();"#
}

/// Spoof the history length.
pub fn spoof_history_length_script(length: u32) -> String {
    format!(
        "(()=>{{const h=new Function('return {length}');Object.defineProperty(h,'toString',{{value:()=>`function get length() {{ [native code] }}`}});Object.defineProperty(History.prototype,'length',{{get:h,configurable:!0}});}})()",
        length = length
    )
}

// spoof unused atm for headless browser settings entry.
// pub const SPOOF_MEDIA: &str = r#"Object.defineProperty(Navigator.prototype,'mediaDevices',{get:()=>({getUserMedia:undefined}),configurable:!0,enumerable:!1}),Object.defineProperty(Navigator.prototype,'webkitGetUserMedia',{get:()=>undefined,configurable:!0,enumerable:!1}),Object.defineProperty(Navigator.prototype,'mozGetUserMedia',{get:()=>undefined,configurable:!0,enumerable:!1}),Object.defineProperty(Navigator.prototype,'getUserMedia',{get:()=>undefined,configurable:!0,enumerable:!1});"#;
