pub const HIDE_WEBGL: &str = r#"(()=>{const v='Google Inc. (NVIDIA)',r='ANGLE (NVIDIA, NVIDIA GeForce GTX 1050 Direct3D11 vs_5_0 ps_5_0, D3D11-27.21.14.5671)',f=p=>p===37445?v:p===37446?r:null;for(const k of['WebGLRenderingContext','WebGL2RenderingContext']){const o=globalThis[k]?.prototype?.getParameter;if(o){Object.defineProperty(globalThis[k].prototype,'getParameter',{value:function(p){const spoof=f(p);return spoof??o.call(this,p);},configurable:true});}}const wrap=W=>function(u,...a){const abs=new URL(u,location.href).toString(),b=`(()=>{const v='${v}',r='${r}',f=p=>p===37445?v:p===37446?r:null;for(const k of['WebGLRenderingContext','WebGL2RenderingContext']){const o=self[k]?.prototype?.getParameter;if(o){Object.defineProperty(self[k].prototype,'getParameter',{value:function(p){const s=f(p);return s??o.call(this,p);},configurable:true});}}fetch("${abs}").then(r=>r.text()).then(t=>(0,eval)(t));})();`;return new W(URL.createObjectURL(new Blob([b],{type:'application/javascript'})),...a)};window.Worker=wrap(window.Worker);window.SharedWorker=wrap(window.SharedWorker);})();"#;
pub const HIDE_WEBGL_MAC: &str = r#"(()=>{const v='Google Inc. (Apple)',r='ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Max, Unspecified Version)',f=p=>p===37445?v:p===37446?r:null;for(const k of['WebGLRenderingContext','WebGL2RenderingContext']){const o=globalThis[k]?.prototype?.getParameter;if(o){Object.defineProperty(globalThis[k].prototype,'getParameter',{value:function(p){const spoof=f(p);return spoof??o.call(this,p);},configurable:true});}}const wrap=W=>function(u,...a){const abs=new URL(u,location.href).toString(),b=`(()=>{const v='${v}',r='${r}',f=p=>p===37445?v:p===37446?r:null;for(const k of['WebGLRenderingContext','WebGL2RenderingContext']){const o=self[k]?.prototype?.getParameter;if(o){Object.defineProperty(self[k].prototype,'getParameter',{value:function(p){const s=f(p);return s??o.call(this,p);},configurable:true});}}fetch("${abs}").then(r=>r.text()).then(t=>(0,eval)(t));})();`;return new W(URL.createObjectURL(new Blob([b],{type:'application/javascript'})),...a)};window.Worker=wrap(window.Worker);window.SharedWorker=wrap(window.SharedWorker);})();"#;

/// Hide webgl inside a web worker.
pub fn hide_webgl_worker_script(vendor: &str, renderer: &str) -> String {
    format!(
        r#"const v='{vendor}',r='{renderer}',f=p=>p===37445?v:p===37446?r:null;for(const k of['WebGLRenderingContext','WebGL2RenderingContext']){{const o=self[k]?.prototype?.getParameter;if(o){{Object.defineProperty(self[k].prototype,'getParameter',{{value:function(p){{const s=f(p);return s??o.call(this,p);}},configurable:true}});}}}}"#,
    )
}

/// Hide the webgl gpu spoof.
pub fn hide_webgl_gpu_spoof(vendor: &str, renderer: &str) -> String {
    let escaped_vendor = vendor.replace('\'', "\\'");
    let escaped_renderer = renderer.replace('\'', "\\'");

    let worker_script = hide_webgl_worker_script(&escaped_vendor, &escaped_renderer);

    format!(
        r#"(()=>{{const v='{vendor}',r='{renderer}',f=p=>p===37445?v:p===37446?r:null;for(const k of['WebGLRenderingContext','WebGL2RenderingContext']){{const o=globalThis[k]?.prototype?.getParameter;if(o){{Object.defineProperty(globalThis[k].prototype,'getParameter',{{value:function(p){{const spoof=f(p);return spoof??o.call(this,p);}},configurable:true}});}}}}const wrap=W=>function(u,...a){{const abs=new URL(u,location.href).toString(),b=`(()=>{{{worker_script};fetch("${{abs}}").then(r=>r.text()).then(t=>(0,eval)(t));}})();`;return new W(URL.createObjectURL(new Blob([b],{{type:'application/javascript'}})),...a)}};window.Worker=wrap(window.Worker);window.SharedWorker=wrap(window.SharedWorker);}})();"#,
        vendor = escaped_vendor,
        renderer = escaped_renderer,
        worker_script = worker_script
    )
}
