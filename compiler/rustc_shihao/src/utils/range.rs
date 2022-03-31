use rustc_span::Span;


pub fn span_str(span:&Span, abs: bool) -> Option<String> {
    let mut span_str = format!("{:?}", span);
    // example: "main.rs:12:5: 17:6 (#0)"
    // remove (#xxx)
    let offset = span_str.find('(').unwrap_or(span_str.len());

    span_str.replace_range((offset-1).., "");
    let mut labels: Vec<&str> = span_str.split(":").collect();
    if labels.len() != 5 {
        return None
    }
    let abs_file = std::fs::canonicalize(labels[0]).unwrap();
    labels[0] = abs_file.as_path().to_str().unwrap();
    return Some(labels.join(":"));
}