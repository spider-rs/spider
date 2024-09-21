use super::html2md::parse_html;
use pretty_assertions::assert_eq;

#[test]
fn test_youtube_simple() {
    let md = parse_html("<iframe src='https://www.youtube.com/embed/zE-dmXZp3nU?wmode=opaque' class='fr-draggable' width='640' height='360'></iframe>");
    assert_eq!(md, "[![Embedded YouTube video](https://img.youtube.com/vi/zE-dmXZp3nU/0.jpg)](https://www.youtube.com/watch?v=zE-dmXZp3nU)")
}

#[test]
fn test_instagram_simple() {
    let md = parse_html("<iframe src='https://www.instagram.com/p/B1BKr9Wo8YX/embed/' width='600' height='600'></iframe>");
    assert_eq!(md, "[![Embedded Instagram post](https://www.instagram.com/p/B1BKr9Wo8YX/media/?size=m)](https://www.instagram.com/p/B1BKr9Wo8YX/embed/)")
}

#[test]
fn test_vkontakte_simple() {
    let md = parse_html("<iframe src='https://vk.com/video_ext.php?oid=-76477496&id=456239454&hash=ebfdc2d386617b97' width='640' height='360' frameborder='0' allowfullscreen></iframe>");
    assert_eq!(md, "[![Embedded VK video](https://st.vk.com/images/icons/video_empty_2x.png)](https://vk.com/video-76477496_456239454)")
}