use std::time::{Duration, Instant};

use windows::Win32::{
    System::{Com::*, Ole::*},
    UI::{Accessibility::*, Input::KeyboardAndMouse::*},
};

struct Com;
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn with_focused<T>(
    work: impl FnOnce(IUIAutomationElement) -> Result<T, String>,
) -> Result<T, String> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }.map_err(|e| e.to_string())?;
    let _com = Com;
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| e.to_string())?;
    let element = unsafe { automation.GetFocusedElement() }.map_err(|e| e.to_string())?;
    if unsafe { element.CurrentIsPassword() }
        .map_err(|e| e.to_string())?
        .as_bool()
    {
        return Err("Dictation is unavailable in password fields.".into());
    }
    let control = unsafe { element.CurrentControlType() }.map_err(|e| e.to_string())?;
    if ![
        UIA_EditControlTypeId,
        UIA_DocumentControlTypeId,
        UIA_ComboBoxControlTypeId,
    ]
    .contains(&control)
        || !unsafe { element.CurrentIsEnabled() }
            .map_err(|e| e.to_string())?
            .as_bool()
    {
        return Err("Focus an editable text field before dictating.".into());
    }
    let read_only = unsafe {
        if let Ok(value) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            value.CurrentIsReadOnly().map(|value| value.as_bool())
        } else {
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .and_then(|text| text.DocumentRange())
                .and_then(|range| range.GetAttributeValue(UIA_IsReadOnlyAttributeId))
                .and_then(|value| windows::Win32::System::Variant::VariantToBoolean(&value))
                .map(|value| value.as_bool())
        }
    }.map_err(|_| "This field does not expose whether it is editable. Copy your last dictation from Settings > Dictation.".to_string())?;
    if read_only {
        return Err("Focus an editable text field before dictating.".into());
    }
    work(element)
}

fn identity(element: &IUIAutomationElement) -> Result<String, String> {
    unsafe {
        let array = element.GetRuntimeId().map_err(|e| e.to_string())?;
        let result = (|| -> windows::core::Result<Vec<i32>> {
            let lower = SafeArrayGetLBound(array, 1)?;
            let upper = SafeArrayGetUBound(array, 1)?;
            let mut values = Vec::new();
            for index in lower..=upper {
                let mut value = 0_i32;
                SafeArrayGetElement(array, &index, (&mut value as *mut i32).cast())?;
                values.push(value);
            }
            Ok(values)
        })();
        let _ = SafeArrayDestroy(array);
        let values = result.map_err(|e| e.to_string())?;
        if values.is_empty() {
            return Err("This text field does not expose an accessibility identity.".into());
        }
        serde_json::to_string(&values).map_err(|e| e.to_string())
    }
}

pub async fn capture() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| with_focused(|element| identity(&element)))
        .await
        .map_err(|e| e.to_string())?
}

/// How long typing waits for the shortcut's modifier keys to be released.
const KEYS_RELEASE_WAIT: Duration = Duration::from_secs(2);
const FIELD_CHANGED: &str =
    "The focused text field changed. Copy your last dictation from Settings > Dictation.";
const KEYS_HELD: &str = "Release the shortcut keys before inserting text. Copy your last dictation from Settings > Dictation.";
/// Nothing reached the field.
const BLOCKED: &str = "Windows blocked the dictation, so nothing was inserted. Elevated apps may block text insertion. Copy your last dictation from Settings > Dictation.";
/// Part of the text may be in the field.
const PARTLY_TYPED: &str = "Windows could not insert the entire dictation. Check the field before copying your last dictation from Settings > Dictation. Elevated apps may block text insertion.";

pub async fn insert(target: String, text: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || type_into(&target, &text))
        .await
        .map_err(|e| e.to_string())?
}

/// The captured field is still the focused, editable one. Any UI
/// Automation failure counts as a change.
fn focused(target: &str) -> bool {
    with_focused(|element| identity(&element)).is_ok_and(|focused| focused == target)
}

fn modifiers_held() -> bool {
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } < 0)
}

fn type_into(target: &str, text: &str) -> Result<(), String> {
    // Fail at once for a field that changed rather than after the key wait.
    if !focused(target) {
        return Err(FIELD_CHANGED.into());
    }
    let deadline = Instant::now() + KEYS_RELEASE_WAIT;
    while modifiers_held() {
        if Instant::now() >= deadline {
            return Err(KEYS_HELD.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let inputs: Vec<INPUT> = text
        .encode_utf16()
        .flat_map(|unit| {
            [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP].map(|flags| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wScan: unit,
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            })
        })
        .collect();
    // Typed keys combine with a modifier pressed since the wait, and focus
    // may have moved during it.
    if modifiers_held() {
        return Err(KEYS_HELD.into());
    }
    if !focused(target) {
        return Err(FIELD_CHANGED.into());
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    match sent {
        sent if sent == inputs.len() => Ok(()),
        // Elevated apps refuse synthesized input from a normal process.
        0 => Err(BLOCKED.into()),
        _ => Err(PARTLY_TYPED.into()),
    }
}

pub async fn press_return(target: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || return_into(&target))
        .await
        .map_err(|e| e.to_string())?
}

/// Return, only while the field the dictation was typed into still has focus. A modifier held
/// would make it Shift+Return (a new line) or another shortcut.
fn return_into(target: &str) -> Result<(), String> {
    let deadline = Instant::now() + KEYS_RELEASE_WAIT;
    while modifiers_held() {
        if Instant::now() >= deadline {
            return Err(KEYS_HELD.into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if !focused(target) {
        return Err(FIELD_CHANGED.into());
    }
    let scan = unsafe { MapVirtualKeyW(u32::from(VK_RETURN.0), MAPVK_VK_TO_VSC) } as u16;
    let inputs = [KEYBD_EVENT_FLAGS(0), KEYEVENTF_KEYUP].map(|flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_RETURN,
                wScan: scan,
                dwFlags: flags,
                ..Default::default()
            },
        },
    });
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
    if sent == 1 {
        // A key left down would repeat.
        unsafe { SendInput(&inputs[1..], std::mem::size_of::<INPUT>() as i32) };
    }
    if sent == inputs.len() {
        Ok(())
    } else {
        // Elevated apps refuse synthesized input from a normal process.
        Err(format!("Windows accepted {sent} of 2 Return key events"))
    }
}
