use interprocess::{
    local_socket::{
        prelude::*, ConnectOptions, Listener, ListenerNonblockingMode, ListenerOptions, Name,
        Stream,
    },
    ConnectWaitMode,
};
use std::{
    io::{self, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub struct InstanceListener {
    listener: Listener,
    // Keep the lock until after the socket is closed/unlinked. A stale socket
    // may be reclaimed only by a process that holds this advisory lock.
    #[cfg(unix)]
    _lock: std::fs::File,
}
impl InstanceListener {
    fn accept(&self) -> io::Result<Stream> {
        self.listener.accept()
    }
}

#[cfg(unix)]
fn user_id() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

#[cfg(unix)]
fn socket_path(data: &Path) -> io::Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let absolute = if data.is_absolute() {
        data.to_owned()
    } else {
        std::env::current_dir()?.join(data)
    };
    let identity = absolute.canonicalize().unwrap_or(absolute);
    let hash = crate::assets::sha(identity.as_os_str().as_bytes());
    // macOS sockaddr_un allows only 104 bytes (Linux 108). Neither a user's
    // application-data path nor macOS's long TMPDIR can safely hold the socket.
    Ok(Path::new("/tmp")
        .join(format!("lolping-lite-{}", user_id()))
        .join(format!("{}.sock", &hash[..32])))
}

#[cfg(unix)]
fn private_socket_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = path.symlink_metadata()?;
    if !metadata.is_dir()
        || metadata.uid() != user_id()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "The LOLPing Lite socket directory must be a private directory owned by this user",
        ));
    }
    Ok(())
}

fn connect(data: &Path) -> io::Result<Stream> {
    let connection = ConnectOptions::new()
        .name(name(data)?)
        .wait_mode(ConnectWaitMode::Timeout(Duration::from_millis(500)))
        .nonblocking_stream(true)
        .connect_sync()?;
    #[cfg(unix)]
    if connection.peer_creds()?.euid() != Some(user_id()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Single-instance peer belongs to another user",
        ));
    }
    Ok(connection)
}

fn write_until(connection: &mut Stream, mut bytes: &[u8], deadline: Instant) -> io::Result<()> {
    while !bytes.is_empty() && Instant::now() < deadline {
        match connection.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    if bytes.is_empty() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Single-instance write timed out",
        ))
    }
}

fn read_available(connection: &mut Stream, bytes: &mut [u8]) -> io::Result<usize> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::io::{AsHandle, AsRawHandle},
            ptr,
        };
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;
        let Stream::NamedPipe(pipe) = &*connection;
        let mut available = 0;
        if unsafe {
            PeekNamedPipe(
                pipe.as_handle().as_raw_handle(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut available,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::BrokenPipe {
                Ok(0)
            } else {
                Err(error)
            };
        }
        // PIPE_NOWAIT returns ERROR_NO_DATA for an empty connected pipe. Rust
        // classifies it as BrokenPipe, which interprocess maps to EOF. Peeking
        // first distinguishes "nothing yet" from a disconnected peer.
        if available == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let count = bytes.len().min(available as usize);
        return connection.read(&mut bytes[..count]);
    }
    #[cfg(not(windows))]
    connection.read(bytes)
}

fn name(data: &Path) -> io::Result<Name<'static>> {
    #[cfg(windows)]
    {
        use interprocess::local_socket::GenericNamespaced;
        let hash = crate::assets::sha(data.to_string_lossy().as_bytes());
        Ok(format!("lolping-lite-{}", &hash[..24])
            .to_ns_name::<GenericNamespaced>()?
            .into_owned())
    }
    #[cfg(unix)]
    {
        use interprocess::local_socket::GenericFilePath;
        Ok(socket_path(data)?
            .to_fs_name::<GenericFilePath>()?
            .into_owned())
    }
}
pub fn notify(data: &Path, command: &str) -> io::Result<()> {
    notify_until(data, command, Instant::now() + Duration::from_secs(3))
}

/// A competing process may have acquired the listener before its event loop is
/// ready to acknowledge requests. Wait briefly for that existing process only.
pub fn notify_when_ready(data: &Path, command: &str) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if notify_until(data, command, deadline).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "The existing LOLPing Lite instance did not become ready",
    ))
}

fn notify_until(data: &Path, command: &str, until: Instant) -> io::Result<()> {
    if !["open", "quit"].contains(&command) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut connection = connect(data)?;
    write_until(&mut connection, format!("{command}\n").as_bytes(), until)?;
    let mut reply = [0u8; 16];
    let mut received = 0;
    while Instant::now() < until && received < reply.len() {
        match read_available(&mut connection, &mut reply[received..]) {
            Ok(n) if n > 0 => {
                received += n;
                if reply[..received].contains(&b'\n') {
                    return if &reply[..received] == b"ok\n" {
                        Ok(())
                    } else {
                        Err(io::Error::other("Instance rejected request"))
                    };
                }
            }
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "Instance did not respond",
    ))
}
pub fn listen(data: &Path) -> io::Result<InstanceListener> {
    std::fs::create_dir_all(data)?;
    #[cfg(unix)]
    let lock = {
        use std::os::unix::fs::OpenOptionsExt;
        let path = socket_path(data)?;
        private_socket_directory(path.parent().unwrap())?;
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path.with_extension("lock"))?;
        lock.try_lock().map_err(|error| {
            io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("LOLPing Lite is already starting or running: {error}"),
            )
        })?;
        lock
    };
    let options = ListenerOptions::new()
        .name(name(data)?)
        .nonblocking(ListenerNonblockingMode::Both)
        .try_overwrite(true);
    #[cfg(windows)]
    let options = {
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
        };
        let descriptor = widestring::U16CString::from_str("D:P(A;;GA;;;OW)").unwrap();
        options.security_descriptor(SecurityDescriptor::deserialize(&descriptor)?)
    };
    let listener = options.create_sync()?;
    Ok(InstanceListener {
        listener,
        #[cfg(unix)]
        _lock: lock,
    })
}
pub fn serve(
    instance: InstanceListener,
    proxy: tao::event_loop::EventLoopProxy<crate::native::UserEvent>,
) {
    std::thread::spawn(move || loop {
        // Refer to the complete guard so the advisory lock lives with the loop.
        let mut connection = match instance.accept() {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => break,
        };
        #[cfg(unix)]
        if connection
            .peer_creds()
            .ok()
            .and_then(|credentials| credentials.euid())
            != Some(user_id())
        {
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut bytes = Vec::new();
        while Instant::now() < deadline && bytes.len() < 32 && !bytes.contains(&b'\n') {
            let mut buf = [0u8; 32];
            match read_available(&mut connection, &mut buf) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => break,
            }
        }
        let event = match bytes.as_slice() {
            b"open\n" => crate::native::UserEvent::OpenSettings,
            b"quit\n" => crate::native::UserEvent::Quit,
            _ => {
                let _ = connection.write_all(b"error\n");
                continue;
            }
        };
        if proxy.send_event(event).is_err() {
            break;
        }
        let _ = write_until(
            &mut connection,
            b"ok\n",
            Instant::now() + Duration::from_millis(300),
        );
    });
}
pub fn wait_for_exit(data: &Path) -> io::Result<()> {
    let until = Instant::now() + Duration::from_secs(8);
    while Instant::now() < until {
        match connect(data) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::BrokenPipe
                ) =>
            {
                return Ok(())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) => {}
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "LOLPing Lite is still shutting down",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_instance_acknowledgement_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let instance = listen(dir.path()).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match instance.accept() {
                    Ok(stream) => break stream,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("accept failed: {error}"),
                }
            };
            let mut request = Vec::new();
            while !request.contains(&b'\n') && Instant::now() < deadline {
                let mut bytes = [0; 16];
                match read_available(&mut stream, &mut bytes) {
                    Ok(0) => break,
                    Ok(n) => request.extend_from_slice(&bytes[..n]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("read failed: {error}"),
                }
            }
            assert_eq!(request, b"open\n");
            write_until(&mut stream, b"o", deadline).unwrap();
            std::thread::sleep(Duration::from_millis(40));
            write_until(&mut stream, b"k\n", deadline).unwrap();
        });
        let result = notify(dir.path(), "open");
        server.join().unwrap();
        result.unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn long_data_paths_use_short_private_sockets() {
        use std::os::unix::ffi::OsStrExt;
        let long = Path::new("/some/really/long/user/data/location").join("a".repeat(240));
        let path = socket_path(&long).unwrap();
        assert!(path.as_os_str().as_bytes().len() < 104);
        assert_ne!(path, socket_path(&long.join("other")).unwrap());
    }
    #[test]
    #[cfg(unix)]
    fn another_listener_cannot_replace_a_live_instance() {
        let dir = tempfile::tempdir().unwrap();
        let first = listen(dir.path()).unwrap();
        assert!(matches!(listen(dir.path()),Err(error) if error.kind()==io::ErrorKind::AddrInUse));
        drop(first);
        let reopened = listen(dir.path()).unwrap();
        drop(reopened);
    }
}
