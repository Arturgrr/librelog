//! Which microphone a dictation opens. A Bluetooth headset's microphone
//! drops the headset into its call profile (HFP/HSP), so whatever it plays
//! falls to call quality while dictating; when the system default input is
//! one and the person never chose a microphone, dictation records from the
//! built-in or wired one instead, as meetings do. Hex's "Use the Mac's
//! microphone instead of Bluetooth" (MIT, Christopher Langton).

/// The microphone a dictation records from, and the Bluetooth default it
/// passed over to do so.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneChoice {
    /// `None` records from the system default.
    pub device: Option<String>,
    pub passed_over: Option<String>,
}

// The microphone opens by the audio provider's device name, which only
// matches the device layer's name on macOS and Windows. On Linux it opens
// through cpal/ALSA, whose names never match PulseAudio's, so a replacement
// could not be opened there and the default is kept (see listener-core).
const SWAPS_BLUETOOTH_DEFAULT_MIC: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// The microphone for `explicit`, the one the person chose if any.
pub(crate) fn choose(
    audio: &dyn anlg_audio::AudioProvider,
    explicit: Option<String>,
    avoid_bluetooth: bool,
) -> MicrophoneChoice {
    if !SWAPS_BLUETOOTH_DEFAULT_MIC {
        return MicrophoneChoice {
            device: explicit,
            passed_over: None,
        };
    }
    resolve(explicit, avoid_bluetooth, stand_in, || {
        audio.list_mic_devices()
    })
}

fn stand_in() -> Option<(String, String)> {
    anlg_audio_device::bluetooth_default_stand_in()
        .map(|stand_in| (stand_in.bluetooth.name, stand_in.wired.name))
}

/// `stand_in` gives the Bluetooth default and its wired replacement, and
/// `listed` the inputs the audio provider can open; each is only read when
/// needed, since listing devices slows the start of a dictation.
fn resolve(
    explicit: Option<String>,
    avoid_bluetooth: bool,
    stand_in: impl FnOnce() -> Option<(String, String)>,
    listed: impl FnOnce() -> Vec<String>,
) -> MicrophoneChoice {
    let kept = MicrophoneChoice {
        device: explicit.clone(),
        passed_over: None,
    };
    if explicit.is_some() || !avoid_bluetooth {
        return kept;
    }
    let Some((bluetooth, wired)) = stand_in() else {
        return kept;
    };
    if !listed().contains(&wired) {
        tracing::warn!(device = %wired, "dictation_bluetooth_mic_replacement_unavailable");
        return kept;
    }
    tracing::info!(device = %wired, "dictation_bluetooth_default_mic_replaced");
    MicrophoneChoice {
        device: Some(wired),
        passed_over: Some(bluetooth),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn airpods() -> Option<(String, String)> {
        Some(("AirPods".into(), "MacBook Pro Microphone".into()))
    }

    /// Only "Current default" passes over a Bluetooth headset, and only for
    /// a microphone the provider can open; a chosen one, even Bluetooth, and
    /// the setting turned off keep what was asked for.
    #[test]
    fn a_bluetooth_default_gives_way_only_when_no_microphone_was_chosen() {
        let listed = || vec!["AirPods".to_string(), "MacBook Pro Microphone".to_string()];
        assert_eq!(
            resolve(None, true, airpods, listed),
            MicrophoneChoice {
                device: Some("MacBook Pro Microphone".into()),
                passed_over: Some("AirPods".into()),
            }
        );
        let kept = |device: Option<&str>| MicrophoneChoice {
            device: device.map(Into::into),
            passed_over: None,
        };
        assert_eq!(
            resolve(Some("AirPods".into()), true, airpods, listed),
            kept(Some("AirPods"))
        );
        assert_eq!(resolve(None, false, airpods, listed), kept(None));
        assert_eq!(
            resolve(None, true, airpods, || vec!["AirPods".into()]),
            kept(None)
        );
    }
}
