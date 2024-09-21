use super::html2md::parse_html;
use pretty_assertions::assert_eq;

#[test]
fn test_tables() {
    let md = parse_html(r#"<table>
  <thead>
    <tr>
      <th scope='col'>Minor1</th>
      <th scope='col'>Minor2</th>
      <th scope='col'>Minor3</th>
      <th scope='col'>Minor4</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td>col1</td>
      <td>col2</td>
      <td>col3</td>
      <td>col4</td>
    </tr>
  </tbody>
</table>"#);

    assert_eq!(md, "\
|Minor1|Minor2|Minor3|Minor4|
|------|------|------|------|
| col1 | col2 | col3 | col4 |");
}

#[test]
fn test_tables_invalid_more_headers() {
    let md = parse_html(r#"<table>
  <thead>
    <tr>
      <th scope='col'>Minor1</th>
      <th scope='col'>Minor2</th>
      <th scope='col'>Minor3</th>
      <th scope='col'>Minor4</th>
      <th scope='col'>Minor5</th>
      <th scope='col'>Minor6</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td>col1</td>
      <td>col2</td>
      <td>col3</td>
      <td>col4</td>
    </tr>
  </tbody>
</table>"#);

    assert_eq!(md, "\
|Minor1|Minor2|Minor3|Minor4|Minor5|Minor6|
|------|------|------|------|------|------|
| col1 | col2 | col3 | col4 |      |      |");
}

#[test]
fn test_tables_invalid_more_rows() {
    let md = parse_html(r#"<table>
  <thead>
    <tr>
      <th scope='col'>Minor1</th>
      <th scope='col'>Minor2</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td>col1</td>
      <td>col2</td>
      <td>col3</td>
      <td>col4</td>
    </tr>
  </tbody>
</table>"#);

    assert_eq!(md, "\
|Minor1|Minor2|    |    |
|------|------|----|----|
| col1 | col2 |col3|col4|");
}

#[test]
fn test_tables_odd_column_width() {
    let md = parse_html(r#"<table>
  <thead>
    <tr>
      <th scope='col'>Minor</th>
      <th scope='col'>Major</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td>col1</td>
      <td>col2</td>
    </tr>
  </tbody>
</table>"#);

    assert_eq!(md, "\
|Minor|Major|
|-----|-----|
|col1 |col2 |");
}

#[test]
fn test_tables_alignment() {
    let md = parse_html(r#"<table>
  <thead>
    <tr>
      <th align='right'>Minor1</th>
      <th align='center'>Minor2</th>
      <th align='right'>Minor3</th>
      <th align='left'>Minor4</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td>col1</td>
      <td>col2</td>
      <td>col3</td>
      <td>col4</td>
    </tr>
  </tbody>
</table>"#);

    assert_eq!(md, "\
|Minor1|Minor2|Minor3|Minor4|
|-----:|:----:|-----:|:-----|
| col1 | col2 | col3 | col4 |");
}

#[test]
fn test_tables_wild_example() {
    let md = parse_html(r#"
<table style="width: 100%;">
    <thead>
    <tr>
        <th>One ring<br></th>
        <th>Patterns<br></th>
        <th>Titanic<br></th>
        <th><br></th>
        <th><br></th>
        <th><br></th>
    </tr>
    </thead>
    <tbody>
    <tr>
        <td style="width: 16.6667%;">One ring to rule them all<br></td>
        <td style="width: 16.6667%;">There's one for the sorrow <br></td>
        <td style="width: 16.6667%;">Roll on, Titanic, roll<br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
    </tr>
    <tr>
        <td style="width: 16.6667%;">One ring to find them<br></td>
        <td style="width: 16.6667%;">And two for the joy<br></td>
        <td style="width: 16.6667%;">You're the pride of White Star Line<br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
    </tr>
    <tr>
        <td style="width: 16.6667%;">One ring to bring them all<br></td>
        <td style="width: 16.6667%;">And three for the girls<br></td>
        <td style="width: 16.6667%;">Roll on, Titanic, roll<br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
    </tr>
    <tr>
        <td style="width: 16.6667%;">And in the darkness bind them<br></td>
        <td style="width: 16.6667%;">And four for the boys<br></td>
        <td style="width: 16.6667%;">Into the mists of time<br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
        <td style="width: 16.6667%;"><br></td>
    </tr>
    </tbody>
</table>"#);

    assert_eq!(md, "\
|          One ring           |         Patterns         |              Titanic              |   |   |   |
|-----------------------------|--------------------------|-----------------------------------|---|---|---|
|  One ring to rule them all  |There's one for the sorrow|      Roll on, Titanic, roll       |   |   |   |
|    One ring to find them    |   And two for the joy    |You're the pride of White Star Line|   |   |   |
| One ring to bring them all  | And three for the girls  |      Roll on, Titanic, roll       |   |   |   |
|And in the darkness bind them|  And four for the boys   |      Into the mists of time       |   |   |   |");
}