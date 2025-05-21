// use https://github.com/spider-rs/headless-browser for ideal default settings.

pub use super::spoof_webgl::{HIDE_WEBGL, HIDE_WEBGL_MAC};
use crate::{configs::AgentOs, spoof_referrer, spoof_webgl::hide_webgl_worker_script};
use rand::Rng;

/// Spoof window.chrome identical.
pub const HIDE_CHROME: &str = r#"(()=>{const d=Object.defineProperty,c=window.chrome||{};function np(){return{}}const m={getFieldTrial:function getFieldTrial(){},getHistogram:function getHistogram(){},getVariationParams:function getVariationParams(){},recordBoolean:function recordBoolean(){},recordCount:function recordCount(){}},tt={nowInMicroseconds:function nowInMicroseconds(){}},fns={app:{get:np},csi:function csi(){},loadTimes:function loadTimes(){},getVariableValue:function getVariableValue(){},send:function send(){}};for(const[k,v]of Object.entries(fns))'get'in v?d(c,k,{get:v.get,enumerable:!0,configurable:!0}):(v.toString=()=>`function ${k}() { [native code] }`,d(c,k,{value:v,enumerable:!0,configurable:!0}));d(c,'metricsPrivate',{value:m,enumerable:!0});d(c,'timeTicks',{value:tt,enumerable:!0});d(c,'runtime',{get:np,set:np,enumerable:!0,configurable:!0});d(window,'chrome',{value:c,writable:!0,enumerable:!0})})();"#;

/// No console methods.
pub const HIDE_CONSOLE: &str =
    "['log','warn','error','info','debug','table', 'dir'].forEach((method)=>{console[method]=()=>{}});";

pub const DISABLE_DIALOGS: &str  = "(()=>{const a=window.alert.toString(),c=window.confirm.toString(),p=window.prompt.toString();window.alert=function alert(){};Object.defineProperty(window.alert,'toString',{value:()=>a,configurable:true});window.confirm=function confirm(){return true};Object.defineProperty(window.confirm,'toString',{value:()=>c,configurable:true});window.prompt=function prompt(){return ''};Object.defineProperty(window.prompt,'toString',{value:()=>p,configurable:true});})();";

/// Hide the webdriver from being enabled. You should not need this if you use the cli args to launch chrome - disabled-features=AutomationEnabled
pub const HIDE_WEBDRIVER: &str = r#"(()=>{const r=Function.prototype.toString,g=()=>false;Function.prototype.toString=function(){return this===g?'function get webdriver() { [native code] }':r.call(this)};Object.defineProperty(Navigator.prototype,'webdriver',{get:g,enumerable:false,configurable:true})})();"#;

/// Navigator include pdfViewerEnabled.
pub const NAVIGATOR_SCRIPT: &str = r#"(()=>{const nativeGet=new Function("return true");Object.defineProperty(nativeGet,'toString',{value:()=>"function get pdfViewerEnabled() { [native code] }"});Object.defineProperty(Navigator.prototype,"pdfViewerEnabled",{get:nativeGet,configurable:!0});})();"#;
/// Plugin extension.
pub const PLUGIN_AND_MIMETYPE_SPOOF: &str = r#"(()=>{const m=[{type:'application/pdf',suffixes:'pdf',description:'Portable Document Format'},{type:'text/pdf',suffixes:'pdf',description:'Portable Document Format'}],names=['PDF Viewer','Chrome PDF Viewer','Chromium PDF Viewer','Microsoft Edge PDF Viewer','WebKit built-in PDF'],plugins=[],mimes=[];names.forEach(name=>{const plugin=Object.create(Plugin.prototype);Object.defineProperties(plugin,{name:{value:name},description:{value:'Portable Document Format'},filename:{value:'internal-pdf-viewer'},length:{value:2}});const mt1=Object.create(MimeType.prototype),mt2=Object.create(MimeType.prototype);Object.defineProperties(mt1,{type:{value:m[0].type},suffixes:{value:m[0].suffixes},description:{value:m[0].description},enabledPlugin:{value:plugin}});Object.defineProperties(mt2,{type:{value:m[1].type},suffixes:{value:m[1].suffixes},description:{value:m[1].description},enabledPlugin:{value:plugin}});plugin[0]=mt1;plugin[1]=mt2;mimes.push(mt1,mt2);plugins.push(plugin)});Object.defineProperties(PluginArray.prototype,{item:{value:function(i){return this[i]||null}},namedItem:{value:function(n){return this[n]||null}},toJSON:{value:function(){return[...Array(this.length)].map((_,i)=>this[i])}}});Object.defineProperties(MimeTypeArray.prototype,{item:{value:function(i){return this[i]||null}},namedItem:{value:function(n){return this[n]||null}}});const pa=Object.create(PluginArray.prototype),ma=Object.create(MimeTypeArray.prototype);plugins.forEach((p,i)=>{Object.defineProperty(pa,i,{value:p,enumerable:true});Object.defineProperty(pa,p.name,{value:p})});Object.defineProperty(pa,'length',{value:plugins.length,enumerable:false});const seen=new Set();mimes.forEach((mt,i)=>{Object.defineProperty(ma,i,{value:mt,enumerable:true});if(!seen.has(mt.type)){seen.add(mt.type);Object.defineProperty(ma,mt.type,{value:mt})}});Object.defineProperty(ma,'length',{value:mimes.length,enumerable:false});const g=(v,n)=>{const f=()=>v;Object.defineProperty(f,'toString',{value:()=>`function get ${n}() { [native code] }`});return f};Object.defineProperties(Navigator.prototype,{plugins:{get:g(pa,'plugins')},mimeTypes:{get:g(ma,'mimeTypes')}})})();"#;
/// Spoof the notifications enabled prompt.
pub const SPOOF_NOTIFICATIONS: &str = r#"(()=>{const a=new Function('return "prompt"');Object.defineProperty(a,'toString',{value:()=>`function get permission() { [native code] }`});Object.defineProperty(Notification,'permission',{get:a,configurable:true});const b=new Function("return function(e){if(e&&e.name==='notifications'){return Promise.resolve(Object.setPrototypeOf({state:'prompt',onchange:null},PermissionStatus.prototype))}return this.__nativeQuery__.apply(this,arguments)}")();Object.defineProperty(b,"toString",{value:()=>`function query() { [native code] }`});navigator.permissions.__nativeQuery__=navigator.permissions.query.bind(navigator.permissions);navigator.permissions.query=b})();"#;
/// Spoof the permissions granted by default.
pub const SPOOF_PERMISSIONS_QUERY: &str = r#"(()=>{const map={accelerometer:"granted","background-fetch":"granted","background-sync":"granted",gyroscope:"granted",magnetometer:"granted","screen-wake-lock":"granted",camera:"prompt","display-capture":"prompt",geolocation:"prompt",microphone:"prompt",midi:"prompt",notifications:"prompt","persistent-storage":"prompt"};const native=navigator.permissions.query.bind(navigator.permissions);Object.defineProperty(navigator.permissions,"query",{value:function(p){if(p&&p.name&&map.hasOwnProperty(p.name)){return Promise.resolve(Object.setPrototypeOf({state:map[p.name],onchange:null},PermissionStatus.prototype))}return native(p)},configurable:true});const g=new Function('return "prompt"');Object.defineProperty(g,"toString",{value:()=>`function get permission() { [native code] }`});Object.defineProperty(Notification,"permission",{get:g,configurable:true})})();"#;

/// Shallow hide-permission spoof. (use SPOOF_PERMISSIONS_QUERY instead.)
pub const HIDE_PERMISSIONS: &str = "(()=>{const originalQuery=window.navigator.permissions.query;window.navigator.permissions.__proto__.query=parameters=>{ return parameters.name === 'notifications' ? Promise.resolve({ state: Notification.permission }) : originalQuery(parameters) }; })();";

/// Patch the default en-US local - WIP.
pub const SPOOF_LANGUAGE: &str = r#"(()=>{const v=['en-US','en'],d=Object.getPrototypeOf(navigator),p='languages',g=function(){return v};g.toString=()=>`function get languages() { [native code] }`;try{Object.defineProperty(d,p,{get:g,enumerable:false,configurable:true})}catch(e){}if(typeof WorkerNavigator!=='undefined'){const wd=WorkerNavigator.prototype;if(wd&&wd!==d){try{Object.defineProperty(wd,p,{get:g,enumerable:false,configurable:true})}catch(e){}}}})();"#;

/// Spoof __pwInitScripts - only required when using playwright.
pub const PW_INIT_SCRIPTS_SPOOF: &str = r#"(()=>{try{if('__pwInitScripts'in window){try{delete window.__pwInitScripts}catch{}try{Object.defineProperty(window,'__pwInitScripts',{get:()=>undefined,set:()=>{},configurable:!0})}catch{}}if('__pwInitScripts'in globalThis){try{delete globalThis.__pwInitScripts}catch{}try{Object.defineProperty(globalThis,'__pwInitScripts',{get:()=>undefined,set:()=>{},configurable:!0})}catch{}}}catch{}})();"#;

/// Spoof the touch screen.
pub fn spoof_touch_screen(mobile: bool) -> &'static str {
    // headless already defaults to a touch screen. Spoof for virtual display and real proxy connections.
    if mobile {
        r#"(()=>{const one=()=>1;Object.defineProperty(one,'toString',{value:()=>`function get maxTouchPoints() { [native code] }`});Object.defineProperties(Navigator.prototype,{maxTouchPoints:{get:one,configurable:true},msMaxTouchPoints:{get:one,configurable:true}});try{window.TouchEvent=class TouchEvent extends UIEvent{};document.createEvent=(type)=>(type==='TouchEvent'?new TouchEvent('touchstart'):(Document.prototype.createEvent.call(document,type)));if(!('ontouchstart'in window)){Object.defineProperty(Window.prototype,'ontouchstart',{value:null,writable:true,enumerable:true,configurable:true})}}catch{}})();"#
    } else {
        r#"(()=>{const zero=()=>0;Object.defineProperty(zero,'toString',{value:()=>`function get maxTouchPoints() { [native code] }`});Object.defineProperties(Navigator.prototype,{maxTouchPoints:{get:zero,configurable:true},msMaxTouchPoints:{get:zero,configurable:true}});try{delete window.TouchEvent;Object.defineProperty(window,'TouchEvent',{get:()=>{throw new ReferenceError("TouchEvent is not defined")},configurable:true});document.createEvent=(type)=>{if(type==='TouchEvent'){throw new Error('NotSupportedError')}return Document.prototype.createEvent.call(document,type)};if('ontouchstart'in window){delete window.ontouchstart}if('ontouchstart'in Window.prototype){delete Window.prototype.ontouchstart;Object.defineProperty(Window.prototype,'ontouchstart',{value:undefined,writable:false,enumerable:false,configurable:false})}}catch{}})();"#
    }
}

/// Spoof Media labels. This will update fake media labels with real ones.
pub fn spoof_media_labels_script(agent_os: AgentOs) -> String {
    let camera_label = match agent_os {
        AgentOs::Mac => "FaceTime HD Camera",
        AgentOs::Windows => "Integrated Webcam",
        AgentOs::Linux | AgentOs::Unknown => "Integrated Camera",
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

    let color_depth = match agent_os {
        AgentOs::Mac | AgentOs::Windows | AgentOs::Linux | AgentOs::Unknown => {
            if simulate_hdr {
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
                AgentOs::Mac | AgentOs::Linux | AgentOs::Windows | AgentOs::Unknown => 1.0,
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
        "(()=>{{const h=new Function('return {length}');Object.defineProperty(h,'toString',{{value:()=>`function get length() {{ [native code] }}`}});Object.defineProperty(History.prototype,'length',{{get:h,configurable:!0}});}})();",
        length = length
    )
}

/// Spoof the hardware concurrency limit.
pub fn spoof_hardware_concurrency(concurrency: usize) -> String {
    format!(
        r#"(()=>{{const c={c};function hardwareConcurrency(){{return c}}hardwareConcurrency.toString=()=>'function get hardwareConcurrency() {{ [native code] }}';const s=()=>{{try{{Object.defineProperty(Navigator.prototype,'hardwareConcurrency',{{get:hardwareConcurrency,enumerable:!0,configurable:!0}})}}catch{{}}try{{Object.defineProperty(WorkerNavigator.prototype,'hardwareConcurrency',{{get:hardwareConcurrency,enumerable:!0,configurable:!0}})}}catch{{}}}};s();}})();"#,
        c = concurrency
    )
}

/// Unified worker hardware and webgl.
pub fn unified_worker_override(concurrency: usize, vendor: &str, renderer: &str) -> String {
    let escaped_vendor = vendor.replace('\'', "\\'");
    let escaped_renderer = renderer.replace('\'', "\\'");

    let hc_worker_script = spoof_hardware_concurrency(concurrency);
    let gpu_worker_script = hide_webgl_worker_script(&escaped_vendor, &escaped_renderer);

    // Combined worker script injection (both spoofs at once)
    let combined_worker_script = format!(
        "{hc_script};{gpu_script};",
        hc_script = hc_worker_script,
        gpu_script = gpu_worker_script
    );

    format!(
        r#"(()=>{{{hc_script};{gpu_script};const wrap=W=>function(u,...a){{const abs=new URL(u,location.href).toString(),b=`(()=>{{{combined_script};fetch("${{abs}}").then(r=>r.text()).then(t=>(0,eval)(t));}})();`;return new W(URL.createObjectURL(new Blob([b],{{type:'application/javascript'}})),...a)}};window.Worker=wrap(window.Worker);window.SharedWorker=wrap(window.SharedWorker);}})();"#,
        hc_script = hc_worker_script,
        gpu_script = gpu_worker_script,
        combined_script = combined_worker_script
    )
}

/// Spoof the referer for the document.
pub fn spoof_referer_script(referer: &str) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let value = esc(referer);
    format!(
        "(()=>{{var r=Object.getOwnPropertyDescriptor(Document.prototype,'referrer');Object.defineProperty(document,'referrer',{{get:function(){{return \"{val}\"}},configurable:!0}});Object.defineProperty(Object.getOwnPropertyDescriptor(document,'referrer').get,'toString',{{value:function(){{return 'function get referrer() {{ [native code] }}'}},configurable:!0}});}})();",
        val = value
    )
}

/// Spoof the referer for the document.
pub fn spoof_referer_script_randomized() -> String {
    spoof_referer_script(spoof_referrer())
}

/// Spoof the referer for the document with google search referencing.
pub fn spoof_referer_script_randomized_domain(domain_parsed: &url::Url) -> String {
    use rand::Rng;
    if rand::rng().random_bool(0.5) {
        spoof_referer_script(
            &crate::spoof_refererer::spoof_referrer_google(&domain_parsed)
                .unwrap_or_else(|| spoof_referrer().into()),
        )
    } else {
        spoof_referer_script(spoof_referrer())
    }
}

// spoof unused atm for headless browser settings entry.
pub const SPOOF_MEDIA: &str = r#"Object.defineProperty(Navigator.prototype,'mediaDevices',{get:()=>({getUserMedia:undefined}),configurable:!0,enumerable:!1}),Object.defineProperty(Navigator.prototype,'webkitGetUserMedia',{get:()=>undefined,configurable:!0,enumerable:!1}),Object.defineProperty(Navigator.prototype,'mozGetUserMedia',{get:()=>undefined,configurable:!0,enumerable:!1}),Object.defineProperty(Navigator.prototype,'getUserMedia',{get:()=>undefined,configurable:!0,enumerable:!1});"#;
