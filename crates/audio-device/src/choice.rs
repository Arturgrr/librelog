//! Which microphone a recording opens when nobody chose one. A Bluetooth
//! headset's microphone drops the headset into its call profile (HFP/HSP),
//! so whatever it plays falls to call quality while recording; when the
//! system default input is one, dictation and Annotate's narration record
//! from the built-in or wired one instead, as meetings do. Hex's "Use the
//! Mac's microphone instead of Bluetooth" (MIT, Christopher Langton).

/// The microphone to open, and the Bluetooth default it passed over to do so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputChoice {
    /// `None` records from the system default.
    pub device: Option<String>,
    pub passed_over: Option<String>,
}

// The microphone opens by the audio provider's device name, which only
// matches the device layer's name on macOS and Windows. On Linux it opens
// through cpal/ALSA, whose names never match PulseAudio's, so a replacement
// could not be opened there and the default is kept (see listener-core).
const SWAPS_BLUETOOTH_DEFAULT_MIC: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// The Bluetooth default input's name and its wired replacement's, where the
/// audio provider can open the replacement by that name.
pub fn bluetooth_default_names() -> Option<(String, String)> {
    if !SWAPS_BLUETOOTH_DEFAULT_MIC {
        return None;
    }
    crate::bluetooth_default_stand_in()
        .map(|stand_in| (stand_in.bluetooth.name, stand_in.wired.name))
}

/// The microphone for `explicit`, the one the person chose if any. `stand_in`
/// gives the Bluetooth default and its wired replacement
/// ([`bluetooth_default_names`]), and `listed` the inputs the audio provider
/// can open; each is only read when needed, since listing devices slows the
/// start of a recording.
pub fn choose_input(
    explicit: Option<String>,
    avoid_bluetooth: bool,
    stand_in: impl FnOnce() -> Option<(String, String)>,
    listed: impl FnOnce() -> Vec<String>,
) -> InputChoice {
    let kept = InputChoice {
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
        tracing::warn!(device = %wired, "bluetooth_mic_replacement_unavailable");
        return kept;
    }
    tracing::info!(device = %wired, "bluetooth_default_mic_replaced");
    InputChoice {
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
            choose_input(None, true, airpods, listed),
            InputChoice {
                device: Some("MacBook Pro Microphone".into()),
                passed_over: Some("AirPods".into()),
            }
        );
        let kept = |device: Option<&str>| InputChoice {
            device: device.map(Into::into),
            passed_over: None,
        };
        assert_eq!(
            choose_input(Some("AirPods".into()), true, airpods, listed),
            kept(Some("AirPods"))
        );
        assert_eq!(choose_input(None, false, airpods, listed), kept(None));
        assert_eq!(
            choose_input(None, true, airpods, || vec!["AirPods".into()]),
            kept(None)
        );
    }
}
