//! Service names from the Bluetooth SIG's Assigned Numbers: the service
//! classes (§3.3, `assigned_numbers/uuids/service_class.yaml`) and the GATT
//! services (§3.4, `assigned_numbers/uuids/service_uuids.yaml`), at
//! <https://bitbucket.org/bluetooth-SIG/public>. Names are as the SIG
//! writes them, except where it writes a code identifier (`OBEXObjectPush`,
//! `HCR_Print`) or a bare acronym (`GAP`, `PANU`): those are spelled out as
//! the SIG titles the profile or service in its specifications.

/// The SIG's Bluetooth Base UUID, after the 16-bit part.
const BASE_SUFFIX: &str = "-0000-1000-8000-00805f9b34fb";

/// The service's SIG name by its UUID; `None` for a vendor's own UUID or
/// one the SIG hasn't assigned.
pub(super) fn service_name(uuid: &str) -> Option<&'static str> {
    let uuid = uuid.to_ascii_lowercase();
    let short = uuid.strip_prefix("0000")?.strip_suffix(BASE_SUFFIX)?;
    let short = u16::from_str_radix(short, 16).ok()?;
    SERVICES
        .binary_search_by_key(&short, |&(id, _)| id)
        .ok()
        .map(|i| SERVICES[i].1)
}

/// 16-bit UUID and name, sorted by UUID.
const SERVICES: &[(u16, &str)] = &[
    (0x1000, "Service Discovery Server"),
    (0x1001, "Browse Group Descriptor"),
    (0x1101, "Serial Port"),
    (0x1102, "LAN Access Using PPP"),
    (0x1103, "Dial-Up Networking"),
    (0x1104, "IrMC Sync"),
    (0x1105, "OBEX Object Push"),
    (0x1106, "OBEX File Transfer"),
    (0x1107, "IrMC Sync Command"),
    (0x1108, "Headset"),
    (0x1109, "Cordless Telephony"),
    (0x110A, "Audio Source"),
    (0x110B, "Audio Sink"),
    (0x110C, "A/V Remote Control Target"),
    (0x110D, "Advanced Audio Distribution"),
    (0x110E, "A/V Remote Control"),
    (0x110F, "A/V Remote Control Controller"),
    (0x1110, "Intercom"),
    (0x1111, "Fax"),
    (0x1112, "Headset Audio Gateway"),
    (0x1113, "WAP"),
    (0x1114, "WAP Client"),
    (0x1115, "PAN User"),
    (0x1116, "PAN Network Access Point"),
    (0x1117, "PAN Group Ad-hoc Network"),
    (0x1118, "Direct Printing"),
    (0x1119, "Reference Printing"),
    (0x111A, "Imaging"),
    (0x111B, "Imaging Responder"),
    (0x111C, "Imaging Automatic Archive"),
    (0x111D, "Imaging Referenced Objects"),
    (0x111E, "Hands-Free"),
    (0x111F, "AG Hands-Free"),
    (0x1120, "Direct Printing Referenced Objects"),
    (0x1121, "Reflected UI"),
    (0x1122, "Basic Printing"),
    (0x1123, "Printing Status"),
    (0x1124, "Human Interface Device"),
    (0x1125, "Hardcopy Cable Replacement"),
    (0x1126, "HCR Print"),
    (0x1127, "HCR Scan"),
    (0x1128, "Common ISDN Access"),
    (0x112D, "SIM Access"),
    (0x112E, "Phonebook Access Client"),
    (0x112F, "Phonebook Access Server"),
    (0x1130, "Phonebook Access Profile"),
    (0x1131, "Headset - HS"),
    (0x1132, "Message Access Server"),
    (0x1133, "Message Notification Server"),
    (0x1134, "Message Access Profile"),
    (0x1135, "GNSS"),
    (0x1136, "GNSS Server"),
    (0x1137, "3D Display"),
    (0x1138, "3D Glasses"),
    (0x1139, "3D Synch Profile"),
    (0x113A, "Multi Profile Specification"),
    (0x113B, "Multi Profile Service"),
    (0x113C, "CTN Access Service"),
    (0x113D, "CTN Notification Service"),
    (0x113E, "Calendar Tasks and Notes Profile"),
    (0x1200, "PnP Information"),
    (0x1201, "Generic Networking"),
    (0x1202, "Generic File Transfer"),
    (0x1203, "Generic Audio"),
    (0x1204, "Generic Telephony"),
    (0x1205, "UPnP Service"),
    (0x1206, "UPnP IP Service"),
    (0x1300, "ESDP UPnP IP PAN"),
    (0x1301, "ESDP UPnP IP LAP"),
    (0x1302, "ESDP UPnP L2CAP"),
    (0x1303, "Video Source"),
    (0x1304, "Video Sink"),
    (0x1305, "Video Distribution"),
    (0x1400, "Health Device"),
    (0x1401, "Health Device Source"),
    (0x1402, "Health Device Sink"),
    (0x1800, "Generic Access"),
    (0x1801, "Generic Attribute"),
    (0x1802, "Immediate Alert"),
    (0x1803, "Link Loss"),
    (0x1804, "Tx Power"),
    (0x1805, "Current Time"),
    (0x1806, "Reference Time Update"),
    (0x1807, "Next DST Change"),
    (0x1808, "Glucose"),
    (0x1809, "Health Thermometer"),
    (0x180A, "Device Information"),
    (0x180D, "Heart Rate"),
    (0x180E, "Phone Alert Status"),
    (0x180F, "Battery"),
    (0x1810, "Blood Pressure"),
    (0x1811, "Alert Notification"),
    (0x1812, "Human Interface Device"),
    (0x1813, "Scan Parameters"),
    (0x1814, "Running Speed and Cadence"),
    (0x1815, "Automation IO"),
    (0x1816, "Cycling Speed and Cadence"),
    (0x1818, "Cycling Power"),
    (0x1819, "Location and Navigation"),
    (0x181A, "Environmental Sensing"),
    (0x181B, "Body Composition"),
    (0x181C, "User Data"),
    (0x181D, "Weight Scale"),
    (0x181E, "Bond Management"),
    (0x181F, "Continuous Glucose Monitoring"),
    (0x1820, "Internet Protocol Support"),
    (0x1821, "Indoor Positioning"),
    (0x1822, "Pulse Oximeter"),
    (0x1823, "HTTP Proxy"),
    (0x1824, "Transport Discovery"),
    (0x1825, "Object Transfer"),
    (0x1826, "Fitness Machine"),
    (0x1827, "Mesh Provisioning"),
    (0x1828, "Mesh Proxy"),
    (0x1829, "Reconnection Configuration"),
    (0x183A, "Insulin Delivery"),
    (0x183B, "Binary Sensor"),
    (0x183C, "Emergency Configuration"),
    (0x183D, "Authorization Control"),
    (0x183E, "Physical Activity Monitor"),
    (0x183F, "Elapsed Time"),
    (0x1840, "Generic Health Sensor"),
    (0x1843, "Audio Input Control"),
    (0x1844, "Volume Control"),
    (0x1845, "Volume Offset Control"),
    (0x1846, "Coordinated Set Identification"),
    (0x1847, "Device Time"),
    (0x1848, "Media Control"),
    (0x1849, "Generic Media Control"),
    (0x184A, "Constant Tone Extension"),
    (0x184B, "Telephone Bearer"),
    (0x184C, "Generic Telephone Bearer"),
    (0x184D, "Microphone Control"),
    (0x184E, "Audio Stream Control"),
    (0x184F, "Broadcast Audio Scan"),
    (0x1850, "Published Audio Capabilities"),
    (0x1851, "Basic Audio Announcement"),
    (0x1852, "Broadcast Audio Announcement"),
    (0x1853, "Common Audio"),
    (0x1854, "Hearing Access"),
    (0x1855, "Telephony and Media Audio"),
    (0x1856, "Public Broadcast Announcement"),
    (0x1857, "Electronic Shelf Label"),
    (0x1858, "Gaming Audio"),
    (0x1859, "Mesh Proxy Solicitation"),
    (0x185A, "Industrial Measurement Device"),
    (0x185B, "Ranging"),
    (0x185C, "HID ISO"),
    (0x185D, "Cookware"),
    (0x185E, "Voice Assistant"),
    (0x185F, "Generic Voice Assistant"),
    (0x1860, "Tire Pressure Monitoring System"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_words_not_identifiers() {
        for &(id, name) in SERVICES {
            let camel = name
                .as_bytes()
                .windows(2)
                .any(|w| w[0].is_ascii_lowercase() && w[1].is_ascii_uppercase());
            let readable = !name.contains('_')
                && (!camel
                    || name.starts_with("IrMC")
                    || name.contains("UPnP")
                    || name.contains("PnP"));
            assert!(readable, "0x{id:04X} {name}");
        }
    }

    #[test]
    fn the_table_is_sorted_for_lookup() {
        assert!(SERVICES.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn services_are_named_by_their_short_uuid() {
        assert_eq!(
            service_name("0000110B-0000-1000-8000-00805F9B34FB"),
            Some("Audio Sink")
        );
        assert_eq!(
            service_name("0000111f-0000-1000-8000-00805f9b34fb"),
            Some("AG Hands-Free")
        );
        assert_eq!(
            service_name("00001812-0000-1000-8000-00805f9b34fb"),
            Some("Human Interface Device")
        );
        assert_eq!(
            service_name("00001105-0000-1000-8000-00805f9b34fb"),
            Some("OBEX Object Push")
        );
        assert_eq!(service_name("0000ffff-0000-1000-8000-00805f9b34fb"), None);
        assert_eq!(service_name("0000zzzz-0000-1000-8000-00805f9b34fb"), None);
        assert_eq!(service_name("6e400001-b5a3-f393-e0a9-e50e24dcca9e"), None);
    }
}
