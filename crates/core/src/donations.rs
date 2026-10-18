//! Only non-sensitive onboarding preferences belong in a project file.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    StreamElements,
    Streamlabs,
    Kofi,
    PayPal,
}
impl Provider {
    pub const ALL: [Self; 4] = [
        Self::StreamElements,
        Self::Streamlabs,
        Self::Kofi,
        Self::PayPal,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::StreamElements => "StreamElements",
            Self::Streamlabs => "Streamlabs",
            Self::Kofi => "Ko-fi",
            Self::PayPal => "PayPal",
        }
    }
    pub fn website(self) -> &'static str {
        match self {
            Self::StreamElements => "https://streamelements.com/",
            Self::Streamlabs => "https://streamlabs.com/",
            Self::Kofi => "https://ko-fi.com/",
            Self::PayPal => "https://www.paypal.com/",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    #[default]
    Choose,
    Prepare,
    Connect,
    Customize,
    Preview,
}
impl Step {
    pub fn index(self) -> usize {
        match self {
            Self::Choose => 0,
            Self::Prepare => 1,
            Self::Connect => 2,
            Self::Customize => 3,
            Self::Preview => 4,
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Choose => Self::Prepare,
            Self::Prepare => Self::Connect,
            Self::Connect => Self::Customize,
            Self::Customize | Self::Preview => Self::Preview,
        }
    }
    pub fn back(self) -> Self {
        match self {
            Self::Choose | Self::Prepare => Self::Choose,
            Self::Connect => Self::Prepare,
            Self::Customize => Self::Connect,
            Self::Preview => Self::Customize,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    pub provider: Provider,
    pub step: Step,
    pub started: bool,
    pub show_name: bool,
    pub show_amount: bool,
    pub show_message: bool,
    pub currency: String,
    pub template: String,
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            provider: Provider::default(),
            step: Step::Choose,
            started: false,
            show_name: true,
            show_amount: true,
            show_message: false,
            currency: "USD".into(),
            template: "Thank you, {user}, for {amount} {currency}!".into(),
        }
    }
}
impl Setup {
    pub fn preview(&self) -> String {
        self.template
            .replace(
                "{user}",
                if self.show_name {
                    "Sample viewer"
                } else {
                    "Anonymous"
                },
            )
            .replace("{amount}", if self.show_amount { "5.00" } else { "" })
            .replace(
                "{currency}",
                if self.show_amount {
                    self.currency.trim()
                } else {
                    ""
                },
            )
            .replace(
                "{message}",
                if self.show_message {
                    "This is a sample donation message."
                } else {
                    ""
                },
            )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_private_and_resume_without_claiming_connection() {
        let mut setup = Setup {
            step: Step::Customize,
            ..Default::default()
        };
        setup.show_name = false;
        setup.show_amount = false;
        let restored: Setup =
            serde_json::from_str(&serde_json::to_string(&setup).unwrap()).unwrap();
        assert_eq!(restored.step, Step::Customize);
        assert!(!restored.preview().contains("Sample viewer"));
        assert!(!restored.preview().contains("5.00"));
    }
}
