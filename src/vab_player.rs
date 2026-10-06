use std::collections::VecDeque;

use anyhow::{Result, ensure};
use bevy::{asset::AssetId, prelude::*};

use crate::vab_asset::{VabAsset, VabAssetHandle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueuedClip {
    clip: usize,
    looping: bool,
}

/// One root player per game object. Ordinary child timelines are baked.
#[derive(Component, Debug)]
pub struct VabPlayer {
    pub current_frame: usize,
    pub playing: bool,
    /// Fractional frame progress in [0, 1).
    pub timer: f64,
    pub frame_rate_override: Option<f32>,
    clip: usize,
    speed: f64,
    looping: bool,
    completed: bool,
    enter_frame: bool,
    queue: VecDeque<QueuedClip>,
    fallback: Option<usize>,
    use_fallback_on_end: bool,
    terminal: bool,
    last_asset: Option<AssetId<VabAsset>>,
}

impl Default for VabPlayer {
    fn default() -> Self {
        Self {
            current_frame: 0,
            playing: true,
            timer: 0.0,
            frame_rate_override: None,
            clip: 0,
            speed: 1.0,
            looping: true,
            completed: false,
            enter_frame: true,
            queue: VecDeque::new(),
            fallback: None,
            use_fallback_on_end: true,
            terminal: false,
            last_asset: None,
        }
    }
}

#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct VabFrameEvent {
    pub entity: Entity,
    pub animation: String,
    pub frame: usize,
    pub name: String,
}

#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct VabCompleteEvent {
    pub entity: Entity,
    pub animation: String,
}

/// Low-level events returned by [`VabPlayer::advance`]. The clip index is
/// captured when the event occurs, even if one update crosses several clips.
#[derive(Debug, PartialEq, Eq)]
pub enum PlaybackEvent {
    Frame {
        clip: usize,
        frame: usize,
        name: String,
    },
    Complete {
        clip: usize,
    },
}

impl VabPlayer {
    pub fn clip(&self) -> usize {
        self.clip
    }

    pub fn is_complete(&self) -> bool {
        self.completed
    }

    pub fn is_looping(&self) -> bool {
        self.looping
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal
    }

    /// Legacy loop control. Prefer `play_loop` and `play_once` in new code.
    pub fn set_looping(&mut self, looping: bool) {
        if !self.terminal {
            self.looping = looping;
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f64) -> Result<()> {
        ensure!(
            speed.is_finite() && speed >= 0.0,
            "speed must be finite and nonnegative"
        );
        self.speed = speed;
        Ok(())
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn resume(&mut self) {
        if !self.completed {
            self.playing = true;
        }
    }

    /// Immediately play a clip using the current legacy loop setting.
    pub fn play(&mut self, asset: &VabAsset, name: &str) -> Result<()> {
        self.ensure_unlocked()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.clear();
        self.use_fallback_on_end = true;
        self.start_clip(clip, self.looping);
        Ok(())
    }

    /// Immediately play and loop a clip. Any pending sequence is replaced.
    pub fn play_loop<'a>(&'a mut self, asset: &VabAsset, name: &str) -> Result<&'a mut Self> {
        self.ensure_unlocked()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.clear();
        self.use_fallback_on_end = false;
        self.start_clip(clip, true);
        Ok(self)
    }

    /// Play once, then run the queued successor or configured fallback.
    pub fn play_once<'a>(&'a mut self, asset: &VabAsset, name: &str) -> Result<&'a mut Self> {
        self.ensure_unlocked()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.clear();
        self.use_fallback_on_end = true;
        self.start_clip(clip, false);
        Ok(self)
    }

    /// Play once and hold the final frame without entering the fallback clip.
    pub fn play_once_and_hold<'a>(
        &'a mut self,
        asset: &VabAsset,
        name: &str,
    ) -> Result<&'a mut Self> {
        self.ensure_unlocked()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.clear();
        self.use_fallback_on_end = false;
        self.start_clip(clip, false);
        Ok(self)
    }

    /// Append a one-shot clip to the current one-shot sequence.
    pub fn then_once<'a>(&'a mut self, asset: &VabAsset, name: &str) -> Result<&'a mut Self> {
        self.ensure_can_queue()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.push_back(QueuedClip {
            clip,
            looping: false,
        });
        Ok(self)
    }

    /// Append the looping final clip of the current sequence.
    pub fn then_loop<'a>(&'a mut self, asset: &VabAsset, name: &str) -> Result<&'a mut Self> {
        self.ensure_can_queue()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.push_back(QueuedClip {
            clip,
            looping: true,
        });
        Ok(self)
    }

    /// Common `action -> idle` shortcut. Both names are validated atomically.
    pub fn play_once_then(
        &mut self,
        asset: &VabAsset,
        once: &str,
        then_loop: &str,
    ) -> Result<&mut Self> {
        self.ensure_unlocked()?;
        let once = resolve_clip(asset, once)?;
        let then_loop = resolve_clip(asset, then_loop)?;
        self.queue.clear();
        self.queue.push_back(QueuedClip {
            clip: then_loop,
            looping: true,
        });
        self.use_fallback_on_end = false;
        self.start_clip(once, false);
        Ok(self)
    }

    /// Set the looping clip used when a one-shot sequence runs out.
    pub fn set_fallback_loop(&mut self, asset: &VabAsset, name: &str) -> Result<&mut Self> {
        self.ensure_unlocked()?;
        self.fallback = Some(resolve_clip(asset, name)?);
        Ok(self)
    }

    pub fn clear_fallback(&mut self) {
        self.fallback = None;
    }

    /// Play a final one-shot animation and lock normal playback changes.
    pub fn play_terminal<'a>(&'a mut self, asset: &VabAsset, name: &str) -> Result<&'a mut Self> {
        self.ensure_unlocked()?;
        let clip = resolve_clip(asset, name)?;
        self.queue.clear();
        self.use_fallback_on_end = false;
        self.terminal = true;
        self.start_clip(clip, false);
        Ok(self)
    }

    /// Remove terminal protection without choosing or starting a clip.
    pub fn reset_terminal(&mut self) {
        self.terminal = false;
    }

    pub fn replay(&mut self) {
        self.current_frame = 0;
        self.timer = 0.0;
        self.completed = false;
        self.enter_frame = true;
        self.playing = true;
    }

    /// Changes the frame without firing events on the skipped interval.
    pub fn seek(&mut self, asset: &VabAsset, frame: usize) -> Result<()> {
        ensure!(
            asset
                .baked
                .clips
                .get(self.clip)
                .is_some_and(|c| frame < c.frames.len()),
            "seek frame out of range"
        );
        self.current_frame = frame;
        self.timer = 0.0;
        self.completed = false;
        self.enter_frame = false;
        Ok(())
    }

    /// Advance playback, carrying excess time through queued clip boundaries.
    pub fn advance(&mut self, asset: &VabAsset, delta_seconds: f64) -> Vec<PlaybackEvent> {
        let mut output = Vec::new();
        if !self.playing || self.completed {
            return output;
        }
        let rate = f64::from(
            self.frame_rate_override
                .unwrap_or_else(|| asset.playback_frame_rate()),
        );
        let mut remaining = delta_seconds * self.speed * rate;
        if !remaining.is_finite() || remaining < 0.0 || !rate.is_finite() || rate <= 0.0 {
            return output;
        }

        while let Some(clip) = asset.baked.clips.get(self.clip) {
            let length = clip.frames.len();
            if length == 0 {
                break;
            }
            let active_clip = self.clip;
            let start = self.current_frame.min(length - 1) as f64 + self.timer;
            let end = start + remaining;
            if !end.is_finite() || end >= u64::MAX as f64 {
                break;
            }

            if self.enter_frame {
                emit_frame_events(clip, active_clip, self.current_frame, &mut output);
                self.enter_frame = false;
            }

            if self.looping {
                let cycles = (end / length as f64).floor() as u64;
                emit_crossed_events(clip, active_clip, start, end, cycles, &mut output);
                let position = end % length as f64;
                self.current_frame = position.floor() as usize;
                self.timer = position.fract();
                break;
            }

            emit_crossed_events(
                clip,
                active_clip,
                start,
                end.min(length as f64),
                0,
                &mut output,
            );
            if end < length as f64 {
                self.current_frame = end.floor() as usize;
                self.timer = end.fract();
                break;
            }

            remaining = end - length as f64;
            output.push(PlaybackEvent::Complete { clip: active_clip });
            let next = self.queue.pop_front().or_else(|| {
                (self.use_fallback_on_end && !self.terminal)
                    .then_some(self.fallback)
                    .flatten()
                    .map(|clip| QueuedClip {
                        clip,
                        looping: true,
                    })
            });
            if let Some(next) = next {
                self.start_clip(next.clip, next.looping);
                // Enter frame zero in this update, even on an exact boundary.
                continue;
            }

            self.current_frame = length - 1;
            self.timer = 0.0;
            self.completed = true;
            self.playing = false;
            break;
        }
        output
    }

    fn start_clip(&mut self, clip: usize, looping: bool) {
        self.clip = clip;
        self.looping = looping;
        self.replay();
    }

    fn ensure_unlocked(&self) -> Result<()> {
        ensure!(
            !self.terminal,
            "VAB player is terminal-locked; call reset_terminal before changing animation"
        );
        Ok(())
    }

    fn ensure_can_queue(&self) -> Result<()> {
        self.ensure_unlocked()?;
        ensure!(!self.looping, "cannot queue after a looping animation");
        ensure!(
            !self.queue.back().is_some_and(|queued| queued.looping),
            "cannot queue after a looping animation"
        );
        Ok(())
    }

    fn reset_for_asset_change(&mut self) {
        let playing = self.playing;
        self.clip = 0;
        self.looping = true;
        self.queue.clear();
        self.fallback = None;
        self.use_fallback_on_end = true;
        self.terminal = false;
        self.replay();
        self.playing = playing;
    }
}

fn resolve_clip(asset: &VabAsset, name: &str) -> Result<usize> {
    asset
        .clip_index(name)
        .ok_or_else(|| anyhow::anyhow!("unknown animation {name}"))
}

fn emit_frame_events(
    clip: &vatf::baked::BakedClip,
    clip_index: usize,
    frame: usize,
    output: &mut Vec<PlaybackEvent>,
) {
    for event in &clip.events {
        if event.frame as usize == frame {
            output.push(PlaybackEvent::Frame {
                clip: clip_index,
                frame,
                name: event.name.clone(),
            });
        }
    }
}

fn emit_crossed_events(
    clip: &vatf::baked::BakedClip,
    clip_index: usize,
    start: f64,
    end: f64,
    cycles: u64,
    output: &mut Vec<PlaybackEvent>,
) {
    if clip.events.is_empty() {
        return;
    }
    let length = clip.frames.len() as f64;
    for cycle in 0..=cycles {
        let offset = cycle as f64 * length;
        for event in &clip.events {
            let position = offset + f64::from(event.frame);
            if position > start && position <= end {
                output.push(PlaybackEvent::Frame {
                    clip: clip_index,
                    frame: event.frame as usize,
                    name: event.name.clone(),
                });
            }
        }
    }
}

pub fn advance_vab_animations(
    time: Res<Time>,
    mut query: Query<(Entity, &VabAssetHandle, &mut VabPlayer)>,
    assets: Res<Assets<VabAsset>>,
    mut frames: MessageWriter<VabFrameEvent>,
    mut completions: MessageWriter<VabCompleteEvent>,
) {
    for (entity, handle, mut player) in &mut query {
        let Some(asset) = assets.get(&handle.0) else {
            continue;
        };
        let id = handle.0.id();
        if player.last_asset.is_some_and(|previous| previous != id) {
            player.reset_for_asset_change();
        }
        player.last_asset = Some(id);
        for event in player.advance(asset, time.delta_secs_f64()) {
            match event {
                PlaybackEvent::Frame { clip, frame, name } => {
                    let Some(animation) = asset.baked.clips.get(clip).map(|clip| clip.name.clone())
                    else {
                        continue;
                    };
                    frames.write(VabFrameEvent {
                        entity,
                        animation,
                        frame,
                        name,
                    });
                }
                PlaybackEvent::Complete { clip } => {
                    let Some(animation) = asset.baked.clips.get(clip).map(|clip| clip.name.clone())
                    else {
                        continue;
                    };
                    completions.write(VabCompleteEvent { entity, animation });
                }
            }
        }
    }
}
