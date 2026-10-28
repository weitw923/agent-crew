//! 定时执行时间窗口。全部使用本机本地时间。

use chrono::{Local, NaiveTime, Timelike};

pub fn parse_clock(raw: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(raw.trim(), "%H:%M").ok()
}

pub fn in_window(start: &str, end: &str, now: NaiveTime) -> bool {
    let (Some(start), Some(end)) = (parse_clock(start), parse_clock(end)) else {
        return true; // 配置损坏时不阻塞任务，避免任务永久无法运行。
    };
    let now = NaiveTime::from_hms_opt(now.hour(), now.minute(), 0).unwrap_or(now);
    if start <= end {
        now >= start && now < end
    } else {
        // 跨午夜窗口，例如 23:00–02:00。
        now >= start || now < end
    }
}

pub fn now_in_window(start: &str, end: &str) -> bool {
    in_window(start, end, Local::now().time())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn normal_and_overnight_windows() {
        assert!(in_window("01:00", "06:00", t(1, 0)));
        assert!(in_window("01:00", "06:00", t(5, 59)));
        assert!(!in_window("01:00", "06:00", t(6, 0)));
        assert!(!in_window("01:00", "06:00", t(0, 59)));
        assert!(in_window("23:00", "02:00", t(23, 30)));
        assert!(in_window("23:00", "02:00", t(1, 30)));
        assert!(!in_window("23:00", "02:00", t(12, 0)));
    }
}
