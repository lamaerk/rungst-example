/// Emit a local-time informational event.
pub fn info(message: &str) {
    println!(
        "{} INFO  {message}",
        chrono::Local::now().format("%H:%M:%S")
    );
}

/// Shorten a public address or signature for display.
pub fn short(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 12 {
        return value.to_owned();
    }
    format!(
        "{}…{}",
        chars[..4].iter().collect::<String>(),
        chars[chars.len() - 4..].iter().collect::<String>()
    )
}
