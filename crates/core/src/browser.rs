use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub enabled: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub audio: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 1280,
            height: 720,
            fps: 30,
            audio: true,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (16..=1920).contains(&self.width)
                && (16..=1920).contains(&self.height)
                && self.width * self.height <= 1920 * 1080,
            "Browser source exceeds 1080p pixel budget"
        );
        anyhow::ensure!((1..=60).contains(&self.fps), "Browser FPS must be 1–60");
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChromaKey {
    pub enabled: bool,
    pub color: [u8; 3],
    pub similarity: f32,
    pub smoothness: f32,
    pub spill: f32,
}
impl Default for ChromaKey {
    fn default() -> Self {
        Self {
            enabled: false,
            color: [0, 255, 0],
            similarity: 0.12,
            smoothness: 0.08,
            spill: 0.2,
        }
    }
}
impl ChromaKey {
    pub fn validate(&self) -> anyhow::Result<()> {
        for v in [self.similarity, self.smoothness, self.spill] {
            anyhow::ensure!(
                v.is_finite() && (0.0..=1.0).contains(&v),
                "Invalid chroma key setting"
            );
        }
        Ok(())
    }
    pub fn shader(&self) -> ([f32; 4], [f32; 2]) {
        let linear = |v: u8| {
            let v = v as f32 / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        (
            [
                linear(self.color[0]),
                linear(self.color[1]),
                linear(self.color[2]),
                if self.enabled { self.similarity } else { -1. },
            ],
            [self.smoothness.max(0.001), self.spill],
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_limits_and_legacy_defaults() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.fps, 30);
        s.validate().unwrap();
        let mut bad = s;
        bad.width = 1921;
        assert!(bad.validate().is_err());
        let k: ChromaKey = serde_json::from_str("{}").unwrap();
        assert!(k.shader().0[3] < 0.);
    }
}
