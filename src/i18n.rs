use std::{
    fmt,
    sync::atomic::{AtomicU8, Ordering},
};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
pub enum Locale {
    #[default]
    #[serde(rename = "en")]
    #[value(name = "en", alias = "en-US")]
    English,
    #[serde(rename = "zh-CN")]
    #[value(name = "zh-CN", alias = "zh")]
    Chinese,
}

impl Locale {
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Chinese => "zh-CN",
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

static CURRENT_LOCALE: AtomicU8 = AtomicU8::new(Locale::English as u8);

pub fn set_locale(locale: Locale) {
    CURRENT_LOCALE.store(locale as u8, Ordering::Relaxed);
}

pub fn locale() -> Locale {
    match CURRENT_LOCALE.load(Ordering::Relaxed) {
        value if value == Locale::Chinese as u8 => Locale::Chinese,
        _ => Locale::English,
    }
}

pub fn text(english: &'static str, chinese: &'static str) -> &'static str {
    match locale() {
        Locale::English => english,
        Locale::Chinese => chinese,
    }
}
