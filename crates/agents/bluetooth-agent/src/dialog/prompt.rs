//! What the dialog says for each request, and how it reads an answer.

use crate::signals::{RequestKind, format_passkey, valid_passkey, valid_pin};

pub(super) const PIN_ERROR: &str = "Enter 1 to 16 letters or digits";
pub(super) const PASSKEY_ERROR: &str = "Enter up to 6 digits";

/// Digits in a passkey.
const PASSKEY_LEN: usize = 6;

/// What the request asks to be typed, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Entry {
    Pin,
    Passkey,
}

pub(super) fn entry(kind: &RequestKind) -> Option<Entry> {
    match kind {
        RequestKind::PinCode => Some(Entry::Pin),
        RequestKind::Passkey => Some(Entry::Passkey),
        _ => None,
    }
}

pub(super) fn title(kind: &RequestKind) -> &'static str {
    match kind {
        RequestKind::Service(_) => "Bluetooth connection",
        _ => "Bluetooth pairing",
    }
}

pub(super) fn message(kind: &RequestKind) -> String {
    match kind {
        RequestKind::PinCode => "Enter the device's PIN.".into(),
        RequestKind::Passkey => "Enter the passkey shown on the device.".into(),
        RequestKind::Confirmation(_) => "Pair if the device shows the same code.".into(),
        RequestKind::Authorization => "This device wants to pair.".into(),
        RequestKind::Service(uuid) => match service_name(uuid) {
            Some(what) => format!("This device wants to connect for {what}."),
            None => format!("This device wants to use service {uuid}."),
        },
        RequestKind::DisplayPinCode(_) => "Enter this PIN on the device.".into(),
        RequestKind::DisplayPasskey { .. } => {
            "Type this passkey on the device, then press Enter.".into()
        }
    }
}

/// The code shown big, if the request has one.
pub(super) fn code(kind: &RequestKind) -> Option<String> {
    match kind {
        RequestKind::Confirmation(passkey) | RequestKind::DisplayPasskey { passkey, .. } => {
            Some(format_passkey(*passkey))
        }
        RequestKind::DisplayPinCode(pin) => Some(pin.clone()),
        _ => None,
    }
}

/// How much of the shown passkey the device has typed.
pub(super) fn progress(kind: &RequestKind) -> Option<String> {
    match kind {
        RequestKind::DisplayPasskey { entered, .. } if *entered > 0 => {
            let typed = usize::from(*entered).min(PASSKEY_LEN);
            Some(format!("{typed} of {PASSKEY_LEN} digits typed"))
        }
        _ => None,
    }
}

/// A typed PIN, or why it can't be sent.
pub(super) fn read_pin(typed: &str) -> Result<String, &'static str> {
    let pin = typed.trim();
    if valid_pin(pin) {
        Ok(pin.to_owned())
    } else {
        Err(PIN_ERROR)
    }
}

/// A typed passkey, or why it can't be sent.
pub(super) fn read_passkey(typed: &str) -> Result<u32, &'static str> {
    let digits = typed.trim();
    if digits.is_empty()
        || digits.len() > PASSKEY_LEN
        || !digits.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(PASSKEY_ERROR);
    }
    digits
        .parse()
        .ok()
        .filter(|p| valid_passkey(*p))
        .ok_or(PASSKEY_ERROR)
}

/// The kind of device, for its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeviceKind {
    Phone,
    Laptop,
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    GameController,
    Pen,
    Display,
    Camera,
    Printer,
}

impl DeviceKind {
    pub(super) const ALL: [DeviceKind; 11] = [
        DeviceKind::Phone,
        DeviceKind::Laptop,
        DeviceKind::Headphones,
        DeviceKind::Speaker,
        DeviceKind::Keyboard,
        DeviceKind::Mouse,
        DeviceKind::GameController,
        DeviceKind::Pen,
        DeviceKind::Display,
        DeviceKind::Camera,
        DeviceKind::Printer,
    ];
}

/// The kind of device by bluetoothd's `Icon`, a freedesktop icon name;
/// `None` for one without its own icon.
pub(super) fn device_kind(icon: &str) -> Option<DeviceKind> {
    Some(match icon {
        "phone" => DeviceKind::Phone,
        "computer" => DeviceKind::Laptop,
        "audio-headset" | "audio-headphones" => DeviceKind::Headphones,
        "audio-card" | "multimedia-player" => DeviceKind::Speaker,
        "input-keyboard" => DeviceKind::Keyboard,
        "input-mouse" => DeviceKind::Mouse,
        "input-gaming" => DeviceKind::GameController,
        "input-tablet" => DeviceKind::Pen,
        "video-display" | "camera-video" => DeviceKind::Display,
        "camera-photo" => DeviceKind::Camera,
        "printer" => DeviceKind::Printer,
        _ => return None,
    })
}

/// What a service is for, in a few words, by its UUID.
pub(super) fn service_name(uuid: &str) -> Option<&'static str> {
    let uuid = uuid.to_ascii_lowercase();
    let short = uuid
        .strip_prefix("0000")
        .and_then(|rest| rest.strip_suffix("-0000-1000-8000-00805f9b34fb"))?;
    Some(match short {
        "1101" => "a serial port",
        "1105" | "1106" => "file transfer",
        "1108" | "1112" => "headset audio",
        "110a" | "110b" | "110d" => "audio",
        "110c" | "110e" | "110f" => "media controls",
        "1115" | "1116" | "1117" => "networking",
        "111e" | "111f" => "phone calls",
        "1124" | "1812" => "input",
        "112d" => "SIM access",
        "112f" | "1130" => "contacts",
        "1132" | "1133" | "1134" => "messages",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_are_named_by_their_short_uuid() {
        assert_eq!(
            service_name("0000110B-0000-1000-8000-00805F9B34FB"),
            Some("audio")
        );
        assert_eq!(
            service_name("0000111f-0000-1000-8000-00805f9b34fb"),
            Some("phone calls")
        );
        assert_eq!(service_name("0000ffff-0000-1000-8000-00805f9b34fb"), None);
        assert_eq!(service_name("6e400001-b5a3-f393-e0a9-e50e24dcca9e"), None);
        assert_eq!(
            message(&RequestKind::Service("x".into())),
            "This device wants to use service x."
        );
    }

    #[test]
    fn devices_are_known_by_their_icon_name() {
        assert_eq!(device_kind("phone"), Some(DeviceKind::Phone));
        assert_eq!(
            device_kind("audio-headphones"),
            Some(DeviceKind::Headphones)
        );
        assert_eq!(device_kind("audio-headset"), Some(DeviceKind::Headphones));
        assert_eq!(device_kind("input-keyboard"), Some(DeviceKind::Keyboard));
        assert_eq!(device_kind("modem"), None);
        assert_eq!(device_kind(""), None);
        // Each kind has its place in ALL.
        for (i, kind) in DeviceKind::ALL.into_iter().enumerate() {
            assert_eq!(kind as usize, i);
        }
    }

    #[test]
    fn passkeys_are_up_to_six_digits() {
        assert_eq!(read_passkey("042"), Ok(42));
        assert_eq!(read_passkey(" 999999 "), Ok(999_999));
        assert_eq!(read_passkey(""), Err(PASSKEY_ERROR));
        assert_eq!(read_passkey("1234567"), Err(PASSKEY_ERROR));
        assert_eq!(read_passkey("12a"), Err(PASSKEY_ERROR));
        assert_eq!(read_passkey("-1"), Err(PASSKEY_ERROR));
    }

    #[test]
    fn pins_are_trimmed_and_checked() {
        assert_eq!(read_pin(" 0000 "), Ok("0000".into()));
        assert_eq!(read_pin("12 34"), Err(PIN_ERROR));
        assert_eq!(read_pin(""), Err(PIN_ERROR));
    }

    #[test]
    fn codes_and_progress() {
        let shown = RequestKind::DisplayPasskey {
            passkey: 42,
            entered: 0,
        };
        assert_eq!(code(&shown).as_deref(), Some("000042"));
        assert_eq!(progress(&shown), None);
        let typing = RequestKind::DisplayPasskey {
            passkey: 42,
            entered: 3,
        };
        assert_eq!(progress(&typing).as_deref(), Some("3 of 6 digits typed"));
        assert_eq!(
            code(&RequestKind::Confirmation(123_456)).as_deref(),
            Some("123456")
        );
        assert_eq!(
            code(&RequestKind::DisplayPinCode("0000".into())).as_deref(),
            Some("0000")
        );
        assert_eq!(code(&RequestKind::Authorization), None);
    }
}
