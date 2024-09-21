use super::html2md::parse_html;
use pretty_assertions::assert_eq;

#[test]
fn test_styles_with_spaces() {
    let md = parse_html(r#"It read:<s> Nobody will ever love you</s>"#);
    assert_eq!(md, r#"It read: ~~Nobody will ever love you~~"#)
}

#[test]
fn test_styles_with_newlines() {
    let md = parse_html(r#"
And she said:<br/>
<s>We are all just prisoners here<br/>
<u> Of our own device<br/>  </s>
And in the master's chambers<br/>
They gathered for the feast<br/>
<em>They stab it with their steely knives</em><br/>
<strong>But they just can't kill the beast<br/></strong>
    
"#);
    assert_eq!(md, "\
And she said:  
~~We are all just prisoners here  
 Of our own device~~  
And in the master's chambers  
They gathered for the feast  
*They stab it with their steely knives*  
**But they just can't kill the beast**")
}