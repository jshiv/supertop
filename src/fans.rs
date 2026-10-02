//! Fan speed sensors.
//!
//! sysinfo does not expose fans, so we read them directly:
//! - macOS: the AppleSMC IOKit service (`F{n}Ac` / `F{n}Mn` / `F{n}Mx` keys)
//! - Linux: `/sys/class/hwmon/*/fan*_input`

#[derive(Clone, Debug)]
pub struct Fan {
    pub label: String,
    pub rpm: f64,
    pub min_rpm: f64,
    pub max_rpm: f64,
}

impl Fan {
    /// 0.0..=1.0 position of the current speed between min and max.
    pub fn ratio(&self) -> f64 {
        if self.max_rpm <= self.min_rpm {
            return if self.rpm > 0.0 { 0.5 } else { 0.0 };
        }
        ((self.rpm - self.min_rpm) / (self.max_rpm - self.min_rpm)).clamp(0.0, 1.0)
    }
}

pub struct FanReader {
    #[cfg(target_os = "macos")]
    smc: Option<smc::Smc>,
}

impl FanReader {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            smc: smc::Smc::open(),
        }
    }

    pub fn read(&mut self) -> Vec<Fan> {
        #[cfg(target_os = "macos")]
        {
            self.smc.as_ref().map(|s| s.fans()).unwrap_or_default()
        }
        #[cfg(target_os = "linux")]
        {
            linux::fans()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Vec::new()
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::Fan;
    use std::fs;

    fn read_num(path: &std::path::Path) -> Option<f64> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    pub fn fans() -> Vec<Fan> {
        let mut out = Vec::new();
        let Ok(dirs) = fs::read_dir("/sys/class/hwmon") else {
            return out;
        };
        let mut dirs: Vec<_> = dirs.flatten().map(|d| d.path()).collect();
        dirs.sort();
        for dir in dirs {
            let chip = fs::read_to_string(dir.join("name")).unwrap_or_default();
            for i in 1..=16 {
                let Some(rpm) = read_num(&dir.join(format!("fan{i}_input"))) else {
                    continue;
                };
                let label = fs::read_to_string(dir.join(format!("fan{i}_label")))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| format!("{} fan{i}", chip.trim()));
                let min_rpm = read_num(&dir.join(format!("fan{i}_min"))).unwrap_or(0.0);
                let max_rpm = read_num(&dir.join(format!("fan{i}_max"))).unwrap_or(5000.0);
                out.push(Fan { label, rpm, min_rpm, max_rpm });
            }
        }
        out
    }
}

#[cfg(target_os = "macos")]
mod smc {
    use super::Fan;
    use std::ffi::{c_char, c_void};

    type KernReturn = i32;
    type IoObject = u32;
    type IoConnect = u32;

    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        fn IOServiceMatching(name: *const c_char) -> *mut c_void;
        fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> IoObject;
        fn IOServiceOpen(service: IoObject, owning_task: u32, typ: u32, conn: *mut IoConnect) -> KernReturn;
        fn IOServiceClose(conn: IoConnect) -> KernReturn;
        fn IOObjectRelease(obj: IoObject) -> KernReturn;
        fn IOConnectCallStructMethod(
            conn: IoConnect,
            selector: u32,
            input: *const c_void,
            input_size: usize,
            output: *mut c_void,
            output_size: *mut usize,
        ) -> KernReturn;
    }

    extern "C" {
        static mach_task_self_: u32;
    }

    const KERNEL_INDEX_SMC: u32 = 2;
    const SMC_CMD_READ_BYTES: u8 = 5;
    const SMC_CMD_READ_KEYINFO: u8 = 9;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Vers {
        major: u8,
        minor: u8,
        build: u8,
        reserved: u8,
        release: u16,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct PLimit {
        version: u16,
        length: u16,
        cpu: u32,
        gpu: u32,
        mem: u32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct KeyInfo {
        data_size: u32,
        data_type: u32,
        data_attributes: u8,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct KeyData {
        key: u32,
        vers: Vers,
        p_limit: PLimit,
        key_info: KeyInfo,
        result: u8,
        status: u8,
        data8: u8,
        data32: u32,
        bytes: [u8; 32],
    }

    const _: () = assert!(std::mem::size_of::<KeyData>() == 80);

    pub struct Smc {
        conn: IoConnect,
    }

    impl Drop for Smc {
        fn drop(&mut self) {
            unsafe {
                IOServiceClose(self.conn);
            }
        }
    }

    fn fourcc(s: &str) -> u32 {
        s.bytes().fold(0u32, |acc, b| (acc << 8) | b as u32)
    }

    impl Smc {
        pub fn open() -> Option<Self> {
            unsafe {
                let matching = IOServiceMatching(c"AppleSMC".as_ptr());
                if matching.is_null() {
                    return None;
                }
                let service = IOServiceGetMatchingService(0, matching);
                if service == 0 {
                    return None;
                }
                let mut conn: IoConnect = 0;
                let kr = IOServiceOpen(service, mach_task_self_, 0, &mut conn);
                IOObjectRelease(service);
                (kr == 0).then_some(Smc { conn })
            }
        }

        fn call(&self, input: &KeyData) -> Option<KeyData> {
            let mut output = KeyData::default();
            let mut out_size = std::mem::size_of::<KeyData>();
            let kr = unsafe {
                IOConnectCallStructMethod(
                    self.conn,
                    KERNEL_INDEX_SMC,
                    input as *const _ as *const c_void,
                    std::mem::size_of::<KeyData>(),
                    &mut output as *mut _ as *mut c_void,
                    &mut out_size,
                )
            };
            (kr == 0 && output.result == 0).then_some(output)
        }

        /// Reads a key and decodes it as a number based on its SMC data type.
        fn read_f64(&self, key: &str) -> Option<f64> {
            let mut input = KeyData { key: fourcc(key), data8: SMC_CMD_READ_KEYINFO, ..Default::default() };
            let info = self.call(&input)?.key_info;
            input.key_info.data_size = info.data_size;
            input.data8 = SMC_CMD_READ_BYTES;
            let out = self.call(&input)?;
            let b = &out.bytes;
            let ty = info.data_type.to_be_bytes();
            match &ty {
                b"flt " => Some(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
                b"fpe2" => Some(u16::from_be_bytes([b[0], b[1]]) as f64 / 4.0),
                b"ui8 " => Some(b[0] as f64),
                b"ui16" => Some(u16::from_be_bytes([b[0], b[1]]) as f64),
                b"ui32" => Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64),
                _ => None,
            }
        }

        pub fn fans(&self) -> Vec<Fan> {
            let count = self.read_f64("FNum").unwrap_or(0.0) as usize;
            (0..count.min(8))
                .filter_map(|i| {
                    let rpm = self.read_f64(&format!("F{i}Ac"))?;
                    Some(Fan {
                        label: match (count, i) {
                            (2, 0) => "Left".into(),
                            (2, 1) => "Right".into(),
                            _ => format!("Fan {}", i + 1),
                        },
                        rpm: rpm.max(0.0),
                        min_rpm: self.read_f64(&format!("F{i}Mn")).unwrap_or(0.0),
                        max_rpm: self.read_f64(&format!("F{i}Mx")).unwrap_or(6000.0),
                    })
                })
                .collect()
        }
    }
}
