//! Far's "Hotplug devices list" (hotplug.cpp): the removable devices that
//! are not "surprise removal OK" (or have such a child), found by walking
//! the device tree from its root; each shown by its description and, when
//! it differs, its friendly name in quotes. Del asks "Device Removal" and
//! ejects it (CM_Request_Device_Eject), then says it can be removed.
//! Outside Windows the list is empty.

#[derive(Clone, Debug)]
pub struct Device {
    pub inst: u32,
    pub description: String,
    pub friendly: String,
}

impl Device {
    /// The menu text (ShowHotplugDevices).
    pub fn label(&self) -> String {
        match (self.description.is_empty(), self.friendly.is_empty()) {
            (false, false) if !self.description.eq_ignore_ascii_case(&self.friendly) => format!("{} \"{}\"", self.description, self.friendly),
            (false, _) => self.description.clone(),
            (true, false) => self.friendly.clone(),
            (true, true) => "UNKNOWN".into(),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::Device;
    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Child, CM_Get_DevNode_Registry_PropertyW, CM_Get_DevNode_Status, CM_Get_Sibling, CM_Locate_DevNodeW, CM_Request_Device_EjectW,
        CM_DEVCAP_DOCKDEVICE, CM_DEVCAP_REMOVABLE, CM_DEVCAP_SURPRISEREMOVALOK, CM_DEVCAP_UNIQUEID, CM_DRP_CAPABILITIES, CM_DRP_DEVICEDESC,
        CM_DRP_FRIENDLYNAME, CM_LOCATE_DEVNODE_NORMAL, CM_PROB_DEVICE_NOT_THERE, CM_PROB_DISABLED, CM_PROB_HELD_FOR_EJECT, CR_SUCCESS,
        PNP_VetoTypeUnknown, PNP_VETO_TYPE,
    };

    fn property_bytes(inst: u32, property: u32) -> Option<Vec<u8>> {
        let mut len: u32 = 0;
        unsafe { CM_Get_DevNode_Registry_PropertyW(inst, property, std::ptr::null_mut(), std::ptr::null_mut(), &mut len, 0) };
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        let ok = unsafe { CM_Get_DevNode_Registry_PropertyW(inst, property, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut len, 0) };
        (ok == CR_SUCCESS).then_some(buf)
    }

    fn capabilities(inst: u32) -> Option<u32> {
        property_bytes(inst, CM_DRP_CAPABILITIES).filter(|b| b.len() >= 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn text(inst: u32, property: u32) -> String {
        let Some(bytes) = property_bytes(inst, property) else { return String::new() };
        let wide: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        String::from_utf16_lossy(&wide).trim().to_string()
    }

    fn child(inst: u32) -> Option<u32> {
        let mut child = 0;
        (unsafe { CM_Get_Child(&mut child, inst, 0) } == CR_SUCCESS).then_some(child)
    }

    /// GetDevicePropertyRecursive: the device's own, else its first child's.
    fn text_recursive(inst: u32, property: u32) -> String {
        let own = text(inst, property);
        if !own.is_empty() {
            return own;
        }
        child(inst).map(|c| text(c, property)).unwrap_or_default()
    }

    fn is_child_hotplug(inst: u32) -> bool {
        child(inst).and_then(capabilities).is_some_and(|caps| caps & CM_DEVCAP_SURPRISEREMOVALOK == 0 && caps & CM_DEVCAP_UNIQUEID != 0)
    }

    fn is_hotplug(inst: u32) -> bool {
        let Some(caps) = capabilities(inst) else { return false };
        let (mut status, mut problem) = (0u32, 0u32);
        if unsafe { CM_Get_DevNode_Status(&mut status, &mut problem, inst, 0) } != CR_SUCCESS {
            return false;
        }
        problem != CM_PROB_DEVICE_NOT_THERE
            && problem != CM_PROB_HELD_FOR_EJECT
            && problem != CM_PROB_DISABLED
            && caps & CM_DEVCAP_REMOVABLE != 0
            && (caps & CM_DEVCAP_SURPRISEREMOVALOK == 0 || is_child_hotplug(inst))
            && caps & CM_DEVCAP_DOCKDEVICE == 0
    }

    fn walk(inst: u32, found: &mut Vec<Device>) {
        if is_hotplug(inst) {
            found.push(Device { inst, description: text_recursive(inst, CM_DRP_DEVICEDESC), friendly: text_recursive(inst, CM_DRP_FRIENDLYNAME) });
        }
        let mut next = child(inst);
        while let Some(c) = next {
            walk(c, found);
            let mut sibling = 0;
            next = (unsafe { CM_Get_Sibling(&mut sibling, c, 0) } == CR_SUCCESS).then_some(sibling);
        }
    }

    pub fn list() -> Vec<Device> {
        let mut root = 0;
        let mut found = Vec::new();
        if unsafe { CM_Locate_DevNodeW(&mut root, std::ptr::null(), CM_LOCATE_DEVNODE_NORMAL) } == CR_SUCCESS {
            walk(root, &mut found);
        }
        found
    }

    pub fn eject(device: &Device) -> bool {
        let mut veto: PNP_VETO_TYPE = PNP_VetoTypeUnknown;
        let mut name = [0u16; 260];
        let result = unsafe { CM_Request_Device_EjectW(device.inst, &mut veto, name.as_mut_ptr(), name.len() as u32, 0) };
        result == CR_SUCCESS && veto == PNP_VetoTypeUnknown
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Device;

    pub fn list() -> Vec<Device> {
        Vec::new()
    }

    pub fn eject(_: &Device) -> bool {
        false
    }
}

pub use platform::{eject, list};
