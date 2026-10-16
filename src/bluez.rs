//! The brush through BlueZ, over D-Bus: discovery, connection, and the one characteristic
//! the protocol uses (0000ff02, write with response + notifications).

use crate::dbus::{Bus, Message, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};

type Res<T> = Result<T, String>;

const BLUEZ: &str = "org.bluez";
const PROPS: &str = "org.freedesktop.DBus.Properties";
const DEVICE: &str = "org.bluez.Device1";
const CHAR: &str = "org.bluez.GattCharacteristic1";
/// The characteristic that carries the protocol, in service 0000ff01.
pub const CHAR_UUID: &str = "0000ff02-0000-1000-8000-00805f9b34fb";

#[derive(Clone, Debug)]
pub struct Device {
    pub path: String,
    pub address: String,
    pub name: String,
    pub rssi: Option<i64>,
}

pub struct Ble {
    bus: Bus,
    adapter: String,
}

/// What `next_notification` saw.
pub enum Event {
    Notification(Vec<u8>),
    Disconnected,
}

impl Ble {
    pub fn open() -> Res<Ble> {
        let mut bus = Bus::system()?;
        let objects = managed_objects(&mut bus)?;
        let mut adapters: Vec<(&str, bool)> = objects
            .entries()
            .into_iter()
            .filter_map(|(path, ifaces)| {
                let a = ifaces.get("org.bluez.Adapter1")?;
                Some((path.as_str()?, a.get("Powered").and_then(Value::as_bool).unwrap_or(false)))
            })
            .collect();
        adapters.sort_by_key(|(path, powered)| (!powered, path.to_string()));
        let (adapter, powered) = adapters.first().ok_or("no Bluetooth adapter (is bluetoothd running?)")?;
        if !powered {
            return Err(format!("the Bluetooth adapter {adapter} is off: bluetoothctl power on"));
        }
        let adapter = adapter.to_string();
        Ok(Ble { bus, adapter })
    }

    fn call(
        &mut self,
        path: &str,
        iface: &str,
        member: &str,
        args: &[Value],
        timeout_ms: u64,
    ) -> Res<Vec<Value>> {
        self.bus.call(BLUEZ, path, iface, member, args, timeout_ms)
    }

    /// A device already connected to this computer, if one matches (a brush that is connected
    /// does not advertise, so discovery would never see it).
    pub fn connected(&mut self, accept: &dyn Fn(&str, &str) -> bool) -> Res<Option<Device>> {
        let objects = managed_objects(&mut self.bus)?;
        for (path, ifaces) in objects.entries() {
            let (Some(path), Some(d)) = (path.as_str(), ifaces.get(DEVICE)) else { continue };
            let name = d.get("Name").and_then(Value::as_str).unwrap_or("");
            let address = d.get("Address").and_then(Value::as_str).unwrap_or("");
            if d.get("Connected").and_then(Value::as_bool) == Some(true) && accept(name, address) {
                return Ok(Some(Device {
                    path: path.into(),
                    address: address.into(),
                    name: name.into(),
                    rssi: None,
                }));
            }
        }
        Ok(None)
    }

    /// Runs a Bluetooth LE discovery for at most `wait`, calling `seen` on every advertising
    /// device that `accept`s (name, address); stops at the first one if `first`.
    pub fn discover(
        &mut self,
        accept: &dyn Fn(&str, &str) -> bool,
        wait: Duration,
        first: bool,
        seen: &mut dyn FnMut(&Device),
    ) -> Res<Vec<Device>> {
        self.bus.add_match(&format!(
            "type='signal',sender='{BLUEZ}',interface='org.freedesktop.DBus.ObjectManager',member='InterfacesAdded'"
        ))?;
        self.bus.add_match(&format!(
            "type='signal',sender='{BLUEZ}',interface='{PROPS}',member='PropertiesChanged',path_namespace='{}'",
            self.adapter
        ))?;
        // Names and addresses of the devices BlueZ already knows: an advertisement from one of
        // them only updates its properties (RSSI, manufacturer data), without the name.
        let mut known: HashMap<String, (String, String)> = HashMap::new();
        for (path, ifaces) in managed_objects(&mut self.bus)?.entries() {
            if let (Some(path), Some(d)) = (path.as_str(), ifaces.get(DEVICE)) {
                known.insert(path.into(), names(d, ("", "")));
            }
        }
        let filter =
            Value::dict(vec![("Transport", Value::Str("le".into())), ("DuplicateData", Value::Bool(true))]);
        let adapter = self.adapter.clone();
        self.call(&adapter, "org.bluez.Adapter1", "SetDiscoveryFilter", &[filter], 5000)?;
        self.call(&adapter, "org.bluez.Adapter1", "StartDiscovery", &[], 5000)?;
        let result = self.watch_adverts(&mut known, accept, wait, first, seen);
        // Always stop, even after an error or a Ctrl+C.
        let _ = self.call(&adapter, "org.bluez.Adapter1", "StopDiscovery", &[], 5000);
        result
    }

    fn watch_adverts(
        &mut self,
        known: &mut HashMap<String, (String, String)>,
        accept: &dyn Fn(&str, &str) -> bool,
        wait: Duration,
        first: bool,
        seen: &mut dyn FnMut(&Device),
    ) -> Res<Vec<Device>> {
        let deadline = Instant::now() + wait;
        let mut found: Vec<Device> = Vec::new();
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Some(m) = self.bus.next_signal(left)? else { break };
            let Some((path, props)) = advert(&m) else { continue };
            let entry = known.entry(path.clone()).or_default();
            *entry = names(props, (&entry.0, &entry.1));
            let (name, address) = entry.clone();
            if address.is_empty() || !accept(&name, &address) || found.iter().any(|d| d.path == path) {
                continue;
            }
            let rssi = props.get("RSSI").and_then(Value::as_i64);
            let d = Device { path, address, name, rssi };
            seen(&d);
            found.push(d);
            if first {
                break;
            }
        }
        Ok(found)
    }

    /// Connects, waits for the GATT services, and turns notifications on. Returns the path of
    /// the protocol characteristic.
    pub fn connect(&mut self, dev: &Device) -> Res<String> {
        self.bus.add_match(&format!(
            "type='signal',sender='{BLUEZ}',interface='{PROPS}',member='PropertiesChanged',path_namespace='{}'",
            dev.path
        ))?;
        match self.call(&dev.path, DEVICE, "Connect", &[], 15_000) {
            Ok(_) => {}
            Err(e) if e.contains("AlreadyConnected") => {}
            // A connection attempt is already under way (ours, from a previous try): wait for it.
            Err(e) if e.contains("InProgress") => {
                if !self.wait_for(&dev.path, "Connected", true, Duration::from_secs(15))? {
                    return Err(format!("could not connect to {}: {e}", dev.address));
                }
            }
            Err(e) => return Err(format!("could not connect to {}: {e}", dev.address)),
        }
        if !self.wait_for(&dev.path, "ServicesResolved", true, Duration::from_secs(15))? {
            return Err("connected, but the brush's services never showed up".into());
        }
        let objects = managed_objects(&mut self.bus)?;
        let chr = objects
            .entries()
            .into_iter()
            .find_map(|(path, ifaces)| {
                let path = path.as_str()?;
                let c = ifaces.get(CHAR)?;
                let uuid = c.get("UUID").and_then(Value::as_str)?;
                (path.starts_with(&format!("{}/", dev.path)) && uuid.eq_ignore_ascii_case(CHAR_UUID))
                    .then(|| path.to_string())
            })
            .ok_or("this device has no 0000ff02 characteristic: not a Laifen brush?")?;
        match self.call(&chr, CHAR, "StartNotify", &[], 10_000) {
            Ok(_) => {}
            Err(e) if e.contains("InProgress") || e.contains("already") => {}
            Err(e) => return Err(format!("could not turn notifications on: {e}")),
        }
        Ok(chr)
    }

    /// Write with response.
    pub fn write(&mut self, chr: &str, bytes: &[u8]) -> Res<()> {
        let opts = Value::dict(vec![("type", Value::Str("request".into()))]);
        self.call(chr, CHAR, "WriteValue", &[Value::bytes(bytes), opts], 10_000).map(|_| ())
    }

    /// The next notification of `chr`, a disconnection of `dev`, or None after `timeout`.
    pub fn next_event(&mut self, dev: &str, chr: &str, timeout: Duration) -> Res<Option<Event>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(m) = self.bus.next_signal(left)? else { return Ok(None) };
            if m.member.as_deref() != Some("PropertiesChanged") {
                continue;
            }
            let (Some(Value::Str(iface)), Some(changed)) = (m.body.first(), m.body.get(1)) else { continue };
            let path = m.path.as_deref().unwrap_or("");
            if path == chr && iface == CHAR {
                if let Some(v) = changed.get("Value").and_then(Value::as_bytes) {
                    return Ok(Some(Event::Notification(v)));
                }
            } else if path == dev
                && iface == DEVICE
                && changed.get("Connected").and_then(Value::as_bool) == Some(false)
            {
                return Ok(Some(Event::Disconnected));
            }
        }
    }

    /// Disconnects (which also cancels a connection attempt still under way), and waits until
    /// BlueZ says so: a command run right after then finds the brush in a clean state.
    pub fn disconnect(&mut self, dev: &str, chr: Option<&str>) {
        if let Some(chr) = chr {
            let _ = self.call(chr, CHAR, "StopNotify", &[], 3000);
        }
        let _ = self.call(dev, DEVICE, "Disconnect", &[], 5000);
        let _ = self.wait_for(dev, "Connected", false, Duration::from_secs(3));
    }

    /// Polls a boolean Device1 property until it equals `want`; false after `timeout`.
    fn wait_for(&mut self, dev: &str, prop: &str, want: bool, timeout: Duration) -> Res<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            let r =
                self.call(dev, PROPS, "Get", &[Value::Str(DEVICE.into()), Value::Str(prop.into())], 5000)?;
            if r.first().and_then(Value::as_bool) == Some(want) {
                return Ok(true);
            }
            if Instant::now() > deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn managed_objects(bus: &mut Bus) -> Res<Value> {
    let mut r =
        bus.call(BLUEZ, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &[], 10_000)?;
    if r.is_empty() {
        return Err("BlueZ returned no objects".into());
    }
    Ok(r.swap_remove(0))
}

/// (name, address) from Device1 properties, keeping `old` values that are not in `props`.
fn names(props: &Value, old: (&str, &str)) -> (String, String) {
    let get = |k: &str, old: &str| props.get(k).and_then(Value::as_str).unwrap_or(old).to_string();
    (get("Name", old.0), get("Address", old.1))
}

/// A device that just advertised: InterfacesAdded with Device1, or PropertiesChanged of
/// Device1 carrying the signal strength or advertised data.
fn advert(m: &Message) -> Option<(String, &Value)> {
    match m.member.as_deref()? {
        "InterfacesAdded" => {
            let path = m.body.first()?.as_str()?;
            Some((path.into(), m.body.get(1)?.get(DEVICE)?))
        }
        "PropertiesChanged" => {
            if m.body.first()?.as_str()? != DEVICE {
                return None;
            }
            let changed = m.body.get(1)?;
            let fresh =
                ["RSSI", "ManufacturerData", "ServiceData", "Name"].iter().any(|k| changed.get(k).is_some());
            fresh.then(|| (m.path.clone().unwrap_or_default(), changed))
        }
        _ => None,
    }
}
