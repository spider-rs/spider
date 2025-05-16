/// Build the gpu main spoof script for WGSLLanguageFeatures and canvas.
pub fn build_gpu_spoof_script_wgsl(canvas_format: &str) -> String {
    format!(
        r#"(() =>{{class WGSLanguageFeatures{{constructor(){{this.size=4}}}}class GPU{{constructor(){{this.wgslLanguageFeatures=new WGSLanguageFeatures()}}requestAdapter(){{return Promise.resolve({{requestDevice:()=>Promise.resolve({{}})}})}}getPreferredCanvasFormat(){{return'{canvas_format}'}}}}const _gpu=new GPU(),_g=()=>_gpu;Object.defineProperty(_g,'toString',{{value:()=>`function get gpu() {{ [native code] }}`,configurable:!0}});Object.defineProperty(Navigator.prototype,'gpu',{{get:_g,configurable:!0,enumerable:!1}});if(typeof WorkerNavigator!=='undefined'){{Object.defineProperty(WorkerNavigator.prototype,'gpu',{{get:_g,configurable:!0,enumerable:!1}})}}}})();"#
    )
}
