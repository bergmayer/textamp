//! Queue icon identities shared by the graphical and embedded terminal adapters.
use crate::app::{presentation::NpSidebarButton, state::RepeatMode};
use gpui_kit::assets::IconName;

pub(super) fn glyph(tool: NpSidebarButton, repeat: RepeatMode) -> IconName {
    use NpSidebarButton::*;
    match tool {
        Shuffle => IconName::Shuffle,
        Repeat if repeat == RepeatMode::Track => IconName::Repeat1,
        Repeat => IconName::Repeat,
        Twins | Twofer => IconName::UsersRound,
        Stretch => IconName::MoveHorizontal,
        Doppelganger => IconName::VenetianMask,
        OppositeDay => IconName::SunMoon,
        Enemies => IconName::Swords,
        ClearQueue => IconName::Trash,
        Save => IconName::Save,
        Undo => IconName::Undo,
        AutoMix => IconName::Blend,
    }
}
