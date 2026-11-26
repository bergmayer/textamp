//! MTP device discovery. The macOS backend reads the USB interface registry.

#[cfg(target_os = "macos")]
pub fn run_worker(location: u32) -> i32 {
    unsafe extern "C" {
        fn textamp_mtp_worker(location: u32) -> i32;
    }
    // The native entry point accepts a USB location by value and owns its run loop.
    unsafe { textamp_mtp_worker(location) }
}

pub(crate) mod session;
/// Read portable profiles and library indexes without claiming a filesystem mount.
pub(crate) async fn volumes() -> anyhow::Result<(Vec<String>, Vec<super::Volume>)> {
    #[cfg(target_os = "macos")]
    {
        let output = tokio::process::Command::new("/usr/sbin/ioreg")
            .args(["-r", "-c", "IOUSBHostDevice", "-d", "2", "-w", "0"])
            .kill_on_drop(true)
            .output()
            .await?;
        anyhow::ensure!(output.status.success(), "Could not enumerate USB devices");
        let tree = String::from_utf8_lossy(&output.stdout);
        let devices = interfaces(&tree);
        crate::app::tasks::spawn_blocking(move || {
            let mut names = vec![];
            let mut volumes = vec![];
            for (name, location) in devices {
                let found = session::scan(location, &name)?;
                if found.is_empty() {
                    names.push(name);
                }
                volumes.extend(found);
            }
            Ok((names, volumes))
        })
        .await?
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok((vec![], vec![]))
    }
}

#[cfg(any(test, target_os = "macos"))]
fn interfaces(tree: &str) -> Vec<(String, u32)> {
    let mut device = None;
    let mut devices = Vec::new();
    for line in tree.lines() {
        let Some((_, node)) = line.split_once("+-o ") else {
            continue;
        };
        let Some((name, class)) = node.split_once("  <class ") else {
            continue;
        };
        if class.starts_with("IOUSBHostDevice,") {
            device = name
                .rsplit_once('@')
                .and_then(|(n, loc)| u32::from_str_radix(loc, 16).ok().map(|l| (n.to_owned(), l)));
        } else if class.starts_with("IOUSBHostInterface,")
            && name
                .split('@')
                .next()
                .is_some_and(|s| s.eq_ignore_ascii_case("MTP"))
            && let Some(d) = device.take()
        {
            devices.push(d);
        }
    }
    devices
}
#[cfg(test)]
mod tests {
    #[test]
    fn identifies_the_location_of_the_device_owning_an_mtp_interface() {
        let tree = "+-o Card Reader@20  <class IOUSBHostDevice, id 5>\n  +-o Mass Storage@0  <class IOUSBHostInterface, id 6>\n+-o Camera@1  <class IOUSBHostDevice, id 1>\n  +-o PTP@0  <class IOUSBHostInterface, id 2>\n+-o Y2@10  <class IOUSBHostDevice, id 3>\n  +-o MTP@0  <class IOUSBHostInterface, id 4>";
        assert_eq!(super::interfaces(tree), vec![("Y2".into(), 16)]);
    }
}
