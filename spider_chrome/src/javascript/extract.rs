pub(crate) const OUTER_HTML: &str = r###"{let rv = ''; if(document.doctype){rv+=new XMLSerializer().serializeToString(document.doctype);} if(document.documentElement){rv+=document.documentElement.outerHTML;} rv}"###;
/// XML serializer for custom pages or testing.
pub(crate) const FULL_XML_SERIALIZER_JS: &str = "(()=>{let e=document.querySelector('#webkit-xml-viewer-source-xml');let x=e?e.innerHTML:new XMLSerializer().serializeToString(document);return x.startsWith('<?xml')?x:'<?xml version=\"1.0\" encoding=\"UTF-8\"?>\\n'+x})()";
