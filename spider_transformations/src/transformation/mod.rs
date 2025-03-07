/// Chunking utils.
pub mod chunking;
/// Content utils.
pub mod content;
/// Text extraction.
pub mod text_extract;

#[cfg(test)]
mod tests {
    use std::vec;

    use crate::transformation::content::{self, ReturnFormat, SelectorConfiguration};
    use maud::PreEscaped;
    use spider::{
        page::build_with_parse,
        tokio::{self, fs::File},
        utils::PageResponse,
    };

    /// the template to re-use
    fn template() -> PreEscaped<String> {
        use maud::{html, DOCTYPE};

        let page_title = "Transform Test";
        let page_h1 = "Fun is fun";

        let markup = html! {
            (DOCTYPE)
            meta charset="utf-8";
            title { (page_title) }
            h1 { (page_h1) }
            a href="https://spider.cloud" { "Spider Cloud"};
            pre {
                r#"The content is ready"#
            }
            script {
                r#"document.querySelector("pre")"#
            }
        };

        markup
    }

    #[test]
    fn test_transformations() {
        let markup = template().into_string();
        let url = "https://spider.cloud";

        let mut conf = content::TransformConfig::default();
        let mut page_response = PageResponse::default();

        page_response.content = Some(Box::new(markup.into()));
        let page = build_with_parse(url, page_response);

        conf.return_format = ReturnFormat::Markdown;

        let content = content::transform_content(&page, &conf, &None, &None, &None);

        assert!(
            content
                .contains(&"Transform Test# Fun is fun\n[Spider Cloud](https://spider.cloud)\n```\nThe content is ready\n```"),
            "The tranform to markdown is invalid"
        );

        conf.return_format = ReturnFormat::Html2Text;

        let content = content::transform_content(&page, &conf, &None, &None, &None);

        assert!(
            content
                .contains(& "# Fun is fun\n\n[Spider Cloud][1]\nThe content is ready\n\n[1]: https://spider.cloud\n"),
            "The tranform to html2text is invalid"
        );

        conf.return_format = ReturnFormat::Bytes;
        conf.readability = true;

        let content = content::transform_content(&page, &conf, &None, &None, &None);

        assert!(
            content
                .contains(&"<html class=\"paper\"><head>\n<meta name=\"disabled-adaptations\" content=\"watch\">\n<meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\n<meta name=\"viewport\" content=\"initial-scale=1\">\n<base href=\"https://spider.cloud/\">\n<title>Transform Test</title>\n<script>window.isReaderPage = true;</script>\n</head><body>\n<h1>Fun is fun</h1><a href=\"https://spider.cloud\">Spider Cloud</a><pre>The content is ready</pre></body></html>"),
            "The tranform to bytes is invalid"
        );

        conf.return_format = ReturnFormat::XML;
        let content = content::transform_content(&page, &conf, &Some("UTF-8".into()), &None, &None);
        assert!(
            content
                == r#"<html xmlns="http://www.w3.org/1999/xhtml" class="paper"><head>
<meta name="disabled-adaptations" content="watch" />
<meta http-equiv="Content-Type" content="text/html; charset=utf-8" />
<meta name="viewport" content="initial-scale=1" />
<base href="https://spider.cloud/" />
<title>Transform Test</title>
<script><![CDATA[window.isReaderPage = true;]]></script>
</head><body>
<h1>Fun is fun</h1><a href="https://spider.cloud">Spider Cloud</a><pre>The content is ready</pre></body></html>"#,
            "The tranform to xml is invalid"
        );
    }

    #[test]
    fn test_xml_transformations() {
        let markup = template().into_string();

        let url = "https://spider.cloud";

        let mut conf = content::TransformConfig::default();
        let mut page_response = PageResponse::default();
        conf.return_format = ReturnFormat::XML;
        page_response.content = Some(Box::new(markup.into()));
        let page = build_with_parse(url, page_response);
        let content = content::transform_content(&page, &conf, &None, &None, &None);
        assert!(
            content
                == r#"<!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><head><meta charset="utf-8" /><title>Transform Test</title></head><body><h1>Fun is fun</h1><a href="https://spider.cloud">Spider Cloud</a><pre>The content is ready</pre><script><![CDATA[document.querySelector(&amp;quot;pre&amp;quot;)]]></script></body></html>"#,
            "The tranform to xml is invalid"
        );
    }

    #[test]
    fn test_transformations_root_selector() {
        let markup = template().into_string();
        let url = "https://spider.cloud";

        let mut conf = content::TransformConfig::default();
        let mut page_response = PageResponse::default();

        page_response.content = Some(Box::new(markup.into()));
        let page = build_with_parse(url, page_response);

        conf.return_format = ReturnFormat::Markdown;

        let mut select_config = SelectorConfiguration::default();

        select_config.root_selector = Some("pre".into());

        let content = content::transform_content(&page, &conf, &None, &Some(select_config), &None);

        assert!(
            content.contains(&"The content is ready"),
            "The tranform to markdown is invalid"
        );
    }

    #[test]
    fn test_transformations_exclude_selector() {
        let markup = template().into_string();
        let url = "https://spider.cloud";

        let mut conf = content::TransformConfig::default();
        let mut page_response = PageResponse::default();

        page_response.content = Some(Box::new(markup.into()));
        let page = build_with_parse(url, page_response);

        conf.return_format = ReturnFormat::Markdown;

        let mut select_config = SelectorConfiguration::default();

        select_config.exclude_selector = Some("pre".into());

        let content = content::transform_content(&page, &conf, &None, &Some(select_config), &None);

        assert!(
            content.contains(&"Transform Test# Fun is fun\n[Spider Cloud](https://spider.cloud)"),
            "The tranform to markdown is invalid"
        );
    }

    #[test]
    fn test_transformations_exclude_selector_text() {
        let markup = template().into_string();
        let url = "https://spider.cloud";

        let mut conf = content::TransformConfig::default();
        let mut page_response = PageResponse::default();

        page_response.content = Some(Box::new(markup.into()));
        let page = build_with_parse(url, page_response);

        conf.return_format = ReturnFormat::Text;

        let mut select_config = SelectorConfiguration::default();

        select_config.exclude_selector = Some("pre".into());

        let content = content::transform_content(&page, &conf, &None, &Some(select_config), &None);

        assert!(
            content.contains(&"Transform Test\nFun is fun Spider Cloud"),
            "The tranform to markdown is invalid"
        );
    }

    #[tokio::test]
    async fn test_transformations_exclude_selector_text_streaming() {
        let markup = template().into_string();
        let url = "https://spider.cloud";

        let mut conf = content::TransformConfig::default();
        let mut page_response = PageResponse::default();

        page_response.content = Some(Box::new(markup.into()));
        let page = build_with_parse(url, page_response);

        conf.return_format = ReturnFormat::Text;

        let mut select_config = SelectorConfiguration::default();

        select_config.exclude_selector = Some("pre".into());

        let content =
            content::transform_content_send(&page, &conf, &None, &Some(select_config), &None).await;

        assert!(
            content.contains(&"Transform Test\nFun is fun Spider Cloud"),
            "The tranform to markdown is invalid"
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_transformations_pdf_handling() {
        use spider::tokio::io::AsyncReadExt;
        let mut f = File::open("./example.pdf").await.unwrap();
        let mut data = vec![];
        f.read_to_end(&mut data).await.unwrap();

        let mut conf = content::TransformConfig::default();
        conf.return_format = ReturnFormat::XML;
        let mut page_response = PageResponse::default();

        page_response.content = Some(Box::new(data));

        let page = build_with_parse("https://example.com/example.pdf", page_response);

        let content = content::transform_content(&page, &conf, &None, &None, &None);

        assert!(content.is_empty(), "The tranform to markdown is invalid");
    }
}
