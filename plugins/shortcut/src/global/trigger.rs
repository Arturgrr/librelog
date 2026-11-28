use tauri_plugin_global_shortcut::{Modifiers, Shortcut};

/// Whether this is a KDE session, whose portal matches triggers by the
/// symbol a key types (see [`trigger_for`]).
pub(super) fn kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktops| {
        desktops
            .split(':')
            .any(|desktop| desktop.eq_ignore_ascii_case("KDE"))
    })
}

pub(super) fn portal_trigger(key: Shortcut) -> Result<String, String> {
    trigger_for(key, kde())
}

/// The symbol Shift makes each key type on a US layout, by its xkb name.
fn shifted(code: &str) -> Option<&'static str> {
    Some(match code {
        "Digit1" => "exclam",
        "Digit2" => "at",
        "Digit3" => "numbersign",
        "Digit4" => "dollar",
        "Digit5" => "percent",
        "Digit6" => "asciicircum",
        "Digit7" => "ampersand",
        "Digit8" => "asterisk",
        "Digit9" => "parenleft",
        "Digit0" => "parenright",
        "Minus" => "underscore",
        "Equal" => "plus",
        "BracketLeft" => "braceleft",
        "BracketRight" => "braceright",
        "Backslash" => "bar",
        "Semicolon" => "colon",
        "Quote" => "quotedbl",
        "Backquote" => "asciitilde",
        "Comma" => "less",
        "Period" => "greater",
        "Slash" => "question",
        _ => return None,
    })
}

/// The trigger the portal is asked for. KWin hands kglobalaccel the symbol a
/// chord types with Shift taken out when Shift made it (Alt+Shift+4 arrives
/// as Alt+$), except for letters, so on KDE `ALT+SHIFT+4` never fires and
/// `ALT+dollar` does. Other layouts put other symbols there; the user can
/// rebind those in System Settings, which Librelog then shows.
pub(super) fn trigger_for(key: Shortcut, kde: bool) -> Result<String, String> {
    let code = key.key.to_string();
    let symbol = (kde && key.mods.contains(Modifiers::SHIFT))
        .then(|| shifted(&code))
        .flatten();
    let mut parts = Vec::new();
    for (flag, label) in [
        (Modifiers::CONTROL, "CTRL"),
        (Modifiers::ALT, "ALT"),
        (Modifiers::SHIFT, "SHIFT"),
        (Modifiers::SUPER, "LOGO"),
    ] {
        if key.mods.contains(flag) && !(flag == Modifiers::SHIFT && symbol.is_some()) {
            parts.push(label.to_string());
        }
    }
    if let Some(symbol) = symbol {
        parts.push(symbol.into());
        return Ok(parts.join("+"));
    }
    parts.push(match code.as_str() {
        "Space" => "space".into(),
        "Backspace" => "BackSpace".into(),
        "Enter" => "Return".into(),
        "NumpadEnter" => "KP_Enter".into(),
        "NumpadAdd" => "KP_Add".into(),
        "NumpadSubtract" => "KP_Subtract".into(),
        "NumpadMultiply" => "KP_Multiply".into(),
        "NumpadDivide" => "KP_Divide".into(),
        "NumpadDecimal" => "KP_Decimal".into(),
        "NumpadComma" => "KP_Separator".into(),
        "NumpadEqual" => "KP_Equal".into(),
        code if code
            .strip_prefix("Numpad")
            .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit()) =>
        {
            format!("KP_{}", &code[6..])
        }
        "ArrowLeft" => "Left".into(),
        "ArrowRight" => "Right".into(),
        "ArrowUp" => "Up".into(),
        "ArrowDown" => "Down".into(),
        "Minus" => "minus".into(),
        "Equal" => "equal".into(),
        "BracketLeft" => "bracketleft".into(),
        "BracketRight" => "bracketright".into(),
        "Backslash" => "backslash".into(),
        "Semicolon" => "semicolon".into(),
        "Quote" => "apostrophe".into(),
        "Backquote" => "grave".into(),
        "Comma" => "comma".into(),
        "Period" => "period".into(),
        "Slash" => "slash".into(),
        "PageUp" => "Page_Up".into(),
        "PageDown" => "Page_Down".into(),
        "CapsLock" => "Caps_Lock".into(),
        "ContextMenu" => "Menu".into(),
        "NumLock" => "Num_Lock".into(),
        "PrintScreen" => "Print".into(),
        "ScrollLock" => "Scroll_Lock".into(),
        "IntlBackslash" => "backslash".into(),
        "IntlYen" => "yen".into(),
        "Tab" | "Escape" | "Delete" | "Insert" | "Home" | "End" | "Pause" => code.clone(),
        // xkb names letters in lower case, as the desktops store them.
        value if value.strip_prefix("Key").is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase()) => value[3..].to_ascii_lowercase(),
        value if value.strip_prefix("Digit").is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit()) => value[5..].into(),
        value if value.strip_prefix('F').and_then(|s| s.parse::<u8>().ok()).is_some_and(|n| (1..=35).contains(&n)) => code.clone(),
        _ => return Err(format!("The {code} key is not supported for Wayland dictation shortcuts. Choose another key.")),
    });
    Ok(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::Code;

    #[test]
    fn translates_portal_keys_without_forwarding_web_code_names() {
        for (key, expected) in [
            (Code::PageUp, "Page_Up"),
            (Code::PageDown, "Page_Down"),
            (Code::CapsLock, "Caps_Lock"),
            (Code::ContextMenu, "Menu"),
            (Code::NumLock, "Num_Lock"),
            (Code::PrintScreen, "Print"),
            (Code::ScrollLock, "Scroll_Lock"),
            (Code::IntlBackslash, "backslash"),
            (Code::IntlYen, "yen"),
            (Code::NumpadComma, "KP_Separator"),
            (Code::NumpadEqual, "KP_Equal"),
            (Code::KeyD, "d"),
            (Code::Digit1, "1"),
            (Code::F12, "F12"),
            (Code::Numpad5, "KP_5"),
            (Code::ArrowLeft, "Left"),
        ] {
            assert_eq!(
                portal_trigger(Shortcut::new(Some(Modifiers::CONTROL), key)).unwrap(),
                format!("CTRL+{expected}")
            );
        }
        assert!(
            portal_trigger(Shortcut::new(Some(Modifiers::CONTROL), Code::Unidentified)).is_err()
        );
    }

    /// Capture's Alt+Shift+4 default never fired on KDE: KWin reports that
    /// chord as Alt+$.
    #[test]
    fn kde_is_asked_for_the_symbol_shift_types_except_for_letters() {
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        for (key, kde, expected) in [
            (Code::Digit4, true, "ALT+dollar"),
            (Code::Slash, true, "ALT+question"),
            (Code::KeyA, true, "ALT+SHIFT+a"),
            (Code::F5, true, "ALT+SHIFT+F5"),
            (Code::Digit4, false, "ALT+SHIFT+4"),
        ] {
            assert_eq!(
                trigger_for(Shortcut::new(Some(alt_shift), key), kde).unwrap(),
                expected
            );
        }
        assert_eq!(
            trigger_for(Shortcut::new(Some(Modifiers::ALT), Code::Digit4), true).unwrap(),
            "ALT+4"
        );
    }
}
