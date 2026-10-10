//! Kubernetes resource quantities (`500m`, `1.5Gi`, `2e3`) as plain numbers.

/// Parses a quantity into base units (cores, bytes, …).
pub fn parse(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '+' | '-')))
        .unwrap_or(s.len());
    let (number, suffix) = s.split_at(split);
    let value: f64 = number.parse().ok()?;
    let multiplier = match suffix {
        "" => 1.0,
        "n" => 1e-9,
        "u" => 1e-6,
        "m" => 1e-3,
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "Ki" => 1024f64,
        "Mi" => 1024f64.powi(2),
        "Gi" => 1024f64.powi(3),
        "Ti" => 1024f64.powi(4),
        "Pi" => 1024f64.powi(5),
        "Ei" => 1024f64.powi(6),
        exp if exp.starts_with(['e', 'E']) => 10f64.powi(exp[1..].parse().ok()?),
        _ => return None,
    };
    Some(value * multiplier)
}

/// CPU quantity in millicores.
pub fn millicores(s: &str) -> f64 {
    parse(s).map_or(0.0, |cores| cores * 1000.0)
}

/// Memory quantity in bytes.
pub fn bytes(s: &str) -> f64 {
    parse(s).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities() {
        assert_eq!(millicores("250m"), 250.0);
        assert_eq!(millicores("2"), 2000.0);
        assert_eq!(millicores("1.5"), 1500.0);
        assert!((millicores("123456789n") - 123.456789).abs() < 1e-9);
        assert_eq!(bytes("1Ki"), 1024.0);
        assert_eq!(bytes("1.5Gi"), 1.5 * 1024f64.powi(3));
        assert_eq!(bytes("128974848"), 128974848.0);
        assert_eq!(bytes("129M"), 129e6);
        assert_eq!(bytes("1e3"), 1000.0);
        assert_eq!(bytes("2E"), 2e18);
        assert_eq!(parse("12Qi"), None);
    }
}
