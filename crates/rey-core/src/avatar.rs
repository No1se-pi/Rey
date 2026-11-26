use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Idle,
    Listening,
    Thinking,
    Speaking,
    Happy,
    Sad,
    Confused,
    Surprised,
    Annoyed,
    Sleeping,
    Error,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "На связи",
            Self::Listening => "Слушает",
            Self::Thinking => "Думает",
            Self::Speaking => "Говорит",
            Self::Happy => "Радуется",
            Self::Sad => "Грустит",
            Self::Confused => "В замешательстве",
            Self::Surprised => "Удивлена",
            Self::Annoyed => "Раздражена",
            Self::Sleeping => "Дремлет",
            Self::Error => "Ошибка",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Manifest {
    pub states: HashMap<State, Clip>,
    #[serde(default)]
    pub fallback_state: State,
    #[serde(default)]
    pub animations: Animations,
    pub canvas: Option<Canvas>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Animations {
    pub blink: Option<Blink>,
    pub speech: Option<Speech>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Blink {
    pub frames: Vec<TimedFrame>,
    pub allowed_states: Vec<State>,
    pub suggested_interval_ms: [u64; 2],
}

#[derive(Clone, Debug, Deserialize)]
pub struct TimedFrame {
    pub file: PathBuf,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Speech {
    pub silence: PathBuf,
    pub frames: SpeechFrames,
    pub suggested_update_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SpeechFrames {
    pub closed: PathBuf,
    pub small: PathBuf,
    pub soft: PathBuf,
    pub round: PathBuf,
}

impl Speech {
    pub fn frame(&self, amplitude: f32) -> &PathBuf {
        match amplitude {
            a if !a.is_finite() || a < 0.015 => &self.silence,
            a if a < 0.055 => &self.frames.small,
            a if a < 0.13 => &self.frames.soft,
            _ => &self.frames.round,
        }
    }
}

impl Blink {
    fn sample(
        &self,
        state: State,
        elapsed: Duration,
        seed: u64,
    ) -> (Option<&PathBuf>, Option<Duration>) {
        if self.frames.is_empty() || !self.allowed_states.contains(&state) {
            return (None, None);
        }
        let frames = self.frames.iter().take(32);
        let duration = |frame: &TimedFrame| frame.duration_ms.clamp(16, 1000);
        let total: u64 = frames.clone().map(duration).sum();
        let low = self.suggested_interval_ms[0].clamp(500, 30_000);
        let high = self.suggested_interval_ms[1].clamp(low, 30_000);
        let mut remaining = elapsed.as_millis().min(3_600_000) as u64;
        let mut random = seed;
        loop {
            let wait = low + (random % (high - low + 1));
            if remaining < wait {
                return (None, Some(Duration::from_millis(wait - remaining)));
            }
            if remaining < wait + total {
                let mut offset = remaining - wait;
                for frame in frames.clone() {
                    let length = duration(frame);
                    if offset < length {
                        return (
                            Some(&frame.file),
                            Some(Duration::from_millis(length - offset)),
                        );
                    }
                    offset -= length;
                }
            }
            remaining -= wait + total;
            random = random.wrapping_add(0x9E3779B97F4A7C15).rotate_right(27);
        }
    }
}

impl Manifest {
    pub fn dynamic_frame(
        &self,
        state: State,
        elapsed: Duration,
        amplitude: f32,
        seed: u64,
    ) -> (Option<&PathBuf>, Option<Duration>) {
        if state == State::Speaking
            && let Some(speech) = &self.animations.speech
        {
            return (Some(speech.frame(amplitude)), None);
        }
        self.animations
            .blink
            .as_ref()
            .map_or((None, None), |blink| blink.sample(state, elapsed, seed))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Clip {
    Png(PathBuf),
    Static {
        file: PathBuf,
    },
    Frames {
        frames: Vec<PathBuf>,
        #[serde(default = "default_fps")]
        fps: f32,
        #[serde(default)]
        mouth: Vec<PathBuf>,
    },
}

fn default_fps() -> f32 {
    12.0
}

impl Clip {
    pub fn frame(&self, elapsed: Duration, amplitude: f32) -> Option<&PathBuf> {
        match self {
            Self::Png(path) | Self::Static { file: path } => Some(path),
            Self::Frames { frames, fps, mouth } => {
                if !mouth.is_empty() && amplitude > 0.02 {
                    let index =
                        ((amplitude.clamp(0.0, 1.0) * 3.0).sqrt() * mouth.len() as f32) as usize;
                    return mouth.get(index.min(mouth.len() - 1));
                }
                let index = (elapsed.as_secs_f64() * f64::from(fps.clamp(1.0, 60.0))) as usize;
                frames.get(index.checked_rem(frames.len())?)
            }
        }
    }
}

pub struct AvatarController {
    state: State,
    changed: Instant,
    pub amplitude: f32,
    pub blink_seed: u64,
}

impl Default for AvatarController {
    fn default() -> Self {
        Self {
            state: State::Idle,
            changed: Instant::now(),
            amplitude: 0.0,
            blink_seed: uuid::Uuid::new_v4().as_u128() as u64,
        }
    }
}

impl AvatarController {
    pub fn set(&mut self, state: State) {
        self.state = state;
        self.changed = Instant::now();
        self.blink_seed = uuid::Uuid::new_v4().as_u128() as u64;
        self.amplitude = 0.0;
    }

    pub fn snapshot(&self) -> (State, Duration) {
        let elapsed = self.changed.elapsed();
        match (self.state, elapsed.as_secs()) {
            (State::Idle, seconds) if seconds >= 180 => {
                (State::Sleeping, elapsed - Duration::from_secs(180))
            }
            (
                State::Idle
                | State::Listening
                | State::Thinking
                | State::Speaking
                | State::Sleeping,
                _,
            ) => (self.state, elapsed),
            (_, seconds) if seconds >= 5 => (State::Idle, elapsed - Duration::from_secs(5)),
            _ => (self.state, elapsed),
        }
    }

    pub fn reply(&mut self, emotion: &str) {
        self.set(match emotion {
            "happy" | "playful" | "warm" => State::Happy,
            "sad" => State::Sad,
            "confused" => State::Confused,
            "surprised" => State::Surprised,
            "annoyed" => State::Annoyed,
            _ => State::Idle,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_manifest_supports_eleven_states_blink_and_lip_sync() {
        let bytes = include_str!("../../../assets/avatar/manifest.json");
        let manifest: Manifest = serde_json::from_str(bytes).unwrap();
        assert_eq!(manifest.states.len(), 11);
        assert_eq!(manifest.canvas.as_ref().unwrap().height, 1402);
        let speech = manifest.animations.speech.as_ref().unwrap();
        assert_eq!(speech.frame(0.0), &PathBuf::from("speech_closed.png"));
        assert_eq!(speech.frame(0.25), &PathBuf::from("speech_round.png"));
        let blink = manifest.animations.blink.as_ref().unwrap();
        assert!(
            blink
                .sample(State::Thinking, Duration::from_secs(5), 0)
                .0
                .is_none()
        );
        assert!(
            blink
                .sample(State::Idle, Duration::from_millis(3110), 0)
                .0
                .is_some()
        );
    }

    #[test]
    fn frames_loop_and_can_follow_volume() {
        let manifest: Manifest = serde_json::from_str(
            r#"{"states":{"speaking":{"frames":["a.png","b.png"],"fps":2,"mouth":["open.png"]}}}"#,
        )
        .unwrap();
        let clip = &manifest.states[&State::Speaking];
        assert_eq!(
            clip.frame(Duration::from_millis(500), 0.0).unwrap(),
            &PathBuf::from("b.png")
        );
        assert_eq!(
            clip.frame(Duration::from_millis(500), 0.4).unwrap(),
            &PathBuf::from("open.png")
        );
        let mut avatar = AvatarController::default();
        avatar.reply("surprised");
        assert_eq!(avatar.snapshot().0, State::Surprised);
    }
}
