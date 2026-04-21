lazy_static! {
    /// Apache server forbidden — DOCTYPE-agnostic suffix shared by all Apache 403 variants.
    pub static ref APACHE_FORBIDDEN_SUFFIX: &'static [u8; 266] = br#"<html><head>
<title>403 Forbidden</title>
</head><body>
<h1>Forbidden</h1>
<p>You don't have permission to access this resource.</p>
<p>Additionally, a 403 Forbidden
error was encountered while trying to use an ErrorDocument to handle the request.</p>
</body></html>"#;

    /// Open Resty forbidden.
    pub static ref OPEN_RESTY_FORBIDDEN: &'static [u8; 125] = br#"<html><head><title>403 Forbidden</title></head>
<body>
<center><h1>403 Forbidden</h1></center>
<hr><center>openresty</center>"#;


  /// Empty HTML.
  pub static ref EMPTY_HTML: &'static [u8; 39] = b"<html><head></head><body></body></html>";
  /// Empty html.
  pub static ref EMPTY_HTML_BASIC: &'static [u8; 13] = b"<html></html>";
}
