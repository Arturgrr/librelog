//! Which microphone a dictation opens: a Bluetooth default gives way to the
//! built-in or wired microphone unless one was chosen
//! ([`anlg_audio_device::choose_input`]).

/// The microphone a dictation records from, and the Bluetooth default it
/// passed over to do so.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneChoice {
    /// `None` records from the system default.
    pub device: Option<String>,
    pub passed_over: Option<String>,
}

/// The microphone for `explicit`, the one the person chose if any.
pub(crate) fn choose(
    audio: &dyn anlg_audio::AudioProvider,
    explicit: Option<String>,
    avoid_bluetooth: bool,
) -> MicrophoneChoice {
    let choice = anlg_audio_device::choose_input(
        explicit,
        avoid_bluetooth,
        anlg_audio_device::bluetooth_default_names,
        || audio.list_mic_devices(),
    );
    MicrophoneChoice {
        device: choice.device,
        passed_over: choice.passed_over,
    }
}
