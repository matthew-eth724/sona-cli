//! One unit, no floats on the boundary. Microseconds everywhere.
use anyhow::{bail, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Time(i64);

impl Time {
    pub const ZERO: Time = Time(0);

    pub fn from_micros(us: i64) -> Time {
        Time(us)
    }
    pub fn micros(self) -> i64 {
        self.0
    }
    /// Rounds. Deepgram sends 0.71999997 for what is really 720ms; truncating
    /// would put every word 1us early.
    pub fn from_secs_f64(s: f64) -> Time {
        Time((s * 1_000_000.0).round() as i64)
    }
    pub fn secs_f64(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
    pub fn saturating_sub(self, o: Time) -> Time {
        Time((self.0 - o.0).max(0))
    }

    /// Accepts `4`, `4s`, `4.5s`, `250ms`, `00:00:04.500`, `1:02.5`.
    /// The fractional part is optional everywhere; Gemini emits both `"3s"`
    /// and `"0.700s"` and a parser that only accepts one of them silently
    /// drops 13% of a transcript.
    pub fn parse(s: &str) -> Result<Time> {
        let t = s.trim();
        if t.is_empty() {
            bail!("empty time");
        }
        fn numeric(x: &str) -> bool {
            !x.is_empty() && x.chars().all(|c| c.is_ascii_digit() || c == '.')
        }
        if t.contains(':') {
            let mut total = 0f64;
            let parts: Vec<&str> = t.split(':').collect();
            if parts.len() > 3 {
                bail!("bad time `{s}`");
            }
            for p in &parts {
                if !numeric(p) {
                    bail!("bad time `{s}`");
                }
                total = total * 60.0 + p.parse::<f64>()?;
            }
            return Ok(Time::from_secs_f64(total));
        }
        // Longest suffix first: `ms` before `s`, and `m`/`h` before the bare
        // number, because `--max-duration 10m` and `2h` are what people type.
        let (num, scale) = if let Some(x) = t.strip_suffix("ms") {
            (x, 0.001)
        } else if let Some(x) = t.strip_suffix('s') {
            (x, 1.0)
        } else if let Some(x) = t.strip_suffix('m') {
            (x, 60.0)
        } else if let Some(x) = t.strip_suffix('h') {
            (x, 3600.0)
        } else {
            (t, 1.0)
        };
        if !numeric(num) {
            bail!("bad time `{s}`");
        }
        Ok(Time::from_secs_f64(num.parse::<f64>()? * scale))
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let us = self.0.max(0);
        let (h, m, s, ms) = (
            us / 3_600_000_000,
            (us / 60_000_000) % 60,
            (us / 1_000_000) % 60,
            (us / 1_000) % 1_000,
        );
        write!(f, "{h:02}:{m:02}:{s:02}.{ms:03}")
    }
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(self.0)
    }
}

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Time, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Micros(i64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Micros(us) => Ok(Time(us)),
            Raw::Text(s) => Time::parse(&s).map_err(serde::de::Error::custom),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

impl Rational {
    pub fn new(num: u32, den: u32) -> Rational {
        Rational {
            num,
            den: den.max(1),
        }
    }
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
    /// `30`, `29.97`, `30000/1001`.
    pub fn parse(s: &str) -> Result<Rational> {
        let t = s.trim();
        if let Some((a, b)) = t.split_once('/') {
            return Ok(Rational::new(a.trim().parse()?, b.trim().parse()?));
        }
        let v: f64 = t.parse()?;
        if (v - 29.97).abs() < 0.001 {
            return Ok(Rational::new(30_000, 1_001));
        }
        if (v - 23.976).abs() < 0.001 {
            return Ok(Rational::new(24_000, 1_001));
        }
        if v <= 0.0 {
            bail!("fps must be positive");
        }
        Ok(Rational::new(v.round() as u32, 1))
    }
}

impl fmt::Display for Rational {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.den == 1 {
            write!(f, "{}", self.num)
        } else {
            write!(f, "{}/{}", self.num, self.den)
        }
    }
}

impl Serialize for Rational {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Rational {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Rational, D::Error> {
        let s = String::deserialize(d)?;
        Rational::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Time,
    pub end: Time,
}

impl TimeRange {
    pub fn new(start: Time, end: Time) -> Result<TimeRange> {
        if end < start {
            bail!("range ends before it starts ({start} .. {end})");
        }
        Ok(TimeRange { start, end })
    }
    pub fn duration(&self) -> Time {
        Time((self.end.0 - self.start.0).max(0))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_documented_form() {
        for (s, expect) in [
            ("4", 4_000_000),
            ("4s", 4_000_000),
            ("4.5s", 4_500_000),
            ("250ms", 250_000),
            ("10m", 600_000_000),
            ("2h", 7_200_000_000),
            ("00:00:04.500", 4_500_000),
            ("1:02.5", 62_500_000),
            // Gemini emits both shapes for the same field.
            ("3s", 3_000_000),
            ("0.700s", 700_000),
        ] {
            assert_eq!(Time::parse(s).unwrap().micros(), expect, "parsing {s}");
        }
    }

    #[test]
    fn rounds_rather_than_truncating() {
        // 0.71999997 * 1e6 truncates to 719_999.
        assert_eq!(Time::from_secs_f64(0.71999997).micros(), 720_000);
    }

    #[test]
    fn rejects_junk() {
        assert!(Time::parse("soon").is_err());
        assert!(Time::parse("").is_err());
        assert!(Time::parse("1:2:3:4").is_err());
    }

    #[test]
    fn rationals_survive_29_97() {
        assert_eq!(Rational::parse("29.97").unwrap(), Rational::new(30_000, 1_001));
        assert_eq!(Rational::parse("30000/1001").unwrap().as_f64().round(), 30.0);
    }
}
