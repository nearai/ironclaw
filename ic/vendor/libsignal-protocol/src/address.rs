use libsignal_protocol_sys as sys;
use std::{
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    os::raw::c_char,
    pin::Pin,
    rc::Rc,
};

/// A reference-counted pointer to a signal address (recipient name, device ID
/// tuple).
#[derive(PartialEq, Eq, Hash)]
pub struct Address(Rc<OwnedAddress>);

impl Address {
    /// Create a new [`Address`].
    pub fn new<N: AsRef<[u8]>>(name: N, device_id: i32) -> Address {
        Address(Rc::new(OwnedAddress::new(name.as_ref(), device_id)))
    }

    /// Create a new [`Address`] from the raw struct.
    ///
    /// # Safety
    ///
    /// The `name` pointed to by the [`sys::signal_protocol_address`] must
    /// outlive this [`Address`].
    pub(crate) unsafe fn from_raw(raw: sys::signal_protocol_address) -> Address {
        let name = std::slice::from_raw_parts(raw.name as *const _, raw.name_len);
        Address::new(name, raw.device_id)
    }

    /// Create an [`Address`] from a pointer to the raw struct.
    ///
    /// # Safety
    ///
    /// (See the notes on [`Address::from_raw`])
    pub(crate) unsafe fn from_ptr(raw: *const sys::signal_protocol_address) -> Address {
        Address::from_raw(raw.read())
    }

    /// Get a string of bytes identifying a recipient (usually their name as a
    /// utf-8 string).
    ///
    /// You may also be looking for the [`Address::as_str`] method.
    pub fn bytes(&self) -> &[u8] {
        self.0.name_bytes()
    }

    /// Get the name attached to this address, converted to a `&str`.
    pub fn as_str(&self) -> Result<&str, std::str::Utf8Error> {
        self.0.name_utf8()
    }

    /// Get the device ID attached to this address.
    pub fn device_id(&self) -> i32 {
        self.0.device_id()
    }

    pub(crate) fn raw(&self) -> &sys::signal_protocol_address {
        &self.0.raw
    }
}

impl Debug for Address {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Clone for Address {
    fn clone(&self) -> Address {
        Address(Rc::clone(&self.0))
    }
}

struct OwnedAddress {
    raw: sys::signal_protocol_address,
    name: Pin<Box<[u8]>>,
}

impl OwnedAddress {
    fn new(name: &[u8], device_id: i32) -> OwnedAddress {
        let name = name.to_vec().into_boxed_slice();
        let name = Pin::new(name);

        OwnedAddress {
            raw: sys::signal_protocol_address {
                name: name.as_ptr() as *const c_char,
                name_len: name.len(),
                device_id,
            },
            name,
        }
    }

    pub fn name_bytes(&self) -> &[u8] {
        &self.name
    }

    pub fn name_utf8(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.name_bytes())
    }

    pub const fn device_id(&self) -> i32 {
        self.raw.device_id
    }
}

impl Debug for OwnedAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut f = f.debug_struct("Address");

        match self.name_utf8() {
            Ok(name) => {
                f.field("name", &name);
            }
            Err(_) => {
                f.field("name", &self.name_bytes());
            }
        }

        f.field("device_id", &self.device_id()).finish()
    }
}

impl Clone for OwnedAddress {
    fn clone(&self) -> OwnedAddress {
        OwnedAddress::new(&self.name, self.raw.device_id)
    }
}

impl PartialEq for OwnedAddress {
    fn eq(&self, other: &OwnedAddress) -> bool {
        self.device_id() == other.device_id() && self.name_bytes() == other.name_bytes()
    }
}

impl Eq for OwnedAddress {}

impl Hash for OwnedAddress {
    fn hash<H: Hasher>(&self, h: &mut H) {
        h.write_i32(self.device_id());
        h.write(self.name_bytes());
    }
}
