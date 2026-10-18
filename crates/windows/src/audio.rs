//! WASAPI shared-mode capture: microphone or desktop loopback, 48 kHz stereo float.
//! Call on a dedicated worker; the callback must not block. No internal audio queue.
use anyhow::{Context, Result, ensure};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_FAILED},
    Media::Audio::*,
    System::{Com::*, Threading::*},
};

#[derive(Clone, Debug)]
pub enum Input {
    Microphone,
    Desktop,
    Endpoint { id: String, loopback: bool },
    Process(u32),
}
pub struct Packet {
    pub samples: Vec<f32>,
    pub position_frames: u64,
    pub timestamp_100ns: u64,
    pub discontinuity: bool,
    pub timestamp_valid: bool,
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Event(HANDLE);
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Started(IAudioClient);
impl Drop for Started {
    fn drop(&mut self) {
        unsafe {
            let _ = self.0.Stop();
        }
    }
}

pub fn run(
    input: Input,
    cancel: &AtomicBool,
    mut deliver: impl FnMut(Packet) -> Result<()>,
) -> Result<()> {
    unsafe {
        // Never change the UI's COM apartment; this function owns its worker apartment.
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .context("Cannot initialize Windows audio worker")?;
        let _apartment = Apartment;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let client: IAudioClient = match &input {
            Input::Process(pid) => process_client(*pid, cancel)?,
            Input::Endpoint { id, .. } => enumerator
                .GetDevice(&windows::core::HSTRING::from(id))?
                .Activate(CLSCTX_ALL, None)?,
            Input::Microphone | Input::Desktop => enumerator
                .GetDefaultAudioEndpoint(
                    if matches!(input, Input::Microphone) {
                        eCapture
                    } else {
                        eRender
                    },
                    eConsole,
                )?
                .Activate(CLSCTX_ALL, None)?,
        };
        let default_flow = match input {
            Input::Microphone => Some(eCapture),
            Input::Desktop => Some(eRender),
            _ => None,
        };
        let initial_default = default_flow
            .and_then(|flow| enumerator.GetDefaultAudioEndpoint(flow, eConsole).ok())
            .and_then(|d| endpoint_id(&d).ok());
        let mut checked_default = std::time::Instant::now();
        let format = WAVEFORMATEX {
            wFormatTag: 3,
            nChannels: 2,
            nSamplesPerSec: 48000,
            nAvgBytesPerSec: 384000,
            nBlockAlign: 8,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        let mut flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
            | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
            | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        if matches!(
            input,
            Input::Desktop | Input::Process(_) | Input::Endpoint { loopback: true, .. }
        ) {
            flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
        }
        client
            .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, &format, None)
            .context("Cannot start 48 kHz Windows audio capture")?;
        let event = Event(CreateEventW(None, false, false, None)?);
        client.SetEventHandle(event.0)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let _started = Started(client);
        while !cancel.load(Ordering::Relaxed) {
            if checked_default.elapsed() >= std::time::Duration::from_secs(1) {
                checked_default = std::time::Instant::now();
                if let Some(flow) = default_flow {
                    let current = enumerator
                        .GetDefaultAudioEndpoint(flow, eConsole)
                        .ok()
                        .and_then(|d| endpoint_id(&d).ok());
                    ensure!(
                        current == initial_default,
                        "Windows default audio device changed; reconnecting"
                    );
                }
            }
            ensure!(
                WaitForSingleObject(event.0, 50) != WAIT_FAILED,
                "Windows audio event wait failed"
            );
            while !cancel.load(Ordering::Relaxed) && capture.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                let mut position = 0;
                let mut timestamp = 0;
                capture.GetBuffer(
                    &mut data,
                    &mut frames,
                    &mut flags,
                    Some(&mut position),
                    Some(&mut timestamp),
                )?;
                struct Release<'a>(&'a IAudioCaptureClient, u32);
                impl Drop for Release<'_> {
                    fn drop(&mut self) {
                        unsafe {
                            let _ = self.0.ReleaseBuffer(self.1);
                        }
                    }
                }
                let release = Release(&capture, frames);
                ensure!(frames <= 48000, "Unexpectedly large Windows audio packet");
                let count = frames as usize * 2;
                let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                    vec![0.0; count]
                } else {
                    ensure!(
                        !data.is_null() || count == 0,
                        "Windows returned empty audio data"
                    );
                    // Read bytes, not potentially unaligned float pointers.
                    if count == 0 {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(data, count * 4)
                            .chunks_exact(4)
                            .map(|b| {
                                let sample = f32::from_le_bytes(b.try_into().unwrap());
                                if sample.is_finite() { sample } else { 0.0 }
                            })
                            .collect()
                    }
                };
                drop(release); // Never retain an OS packet while the mixer processes it.
                deliver(Packet {
                    samples,
                    position_frames: position,
                    timestamp_100ns: timestamp,
                    discontinuity: flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0,
                    timestamp_valid: flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0,
                })?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub capture: bool,
}
fn endpoint_id(device: &IMMDevice) -> Result<String> {
    unsafe {
        let raw = device.GetId()?;
        let result = raw.to_string();
        CoTaskMemFree(Some(raw.0.cast()));
        Ok(result?)
    }
}
pub fn endpoints() -> Result<Vec<Endpoint>> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _apartment = Apartment;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let mut result = Vec::new();
        for (flow, capture) in [(eCapture, true), (eRender, false)] {
            let collection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
            for i in 0..collection.GetCount()? {
                let device = collection.Item(i)?;
                let id = endpoint_id(&device)?;
                let name = device
                    .OpenPropertyStore(STGM_READ)
                    .and_then(|p| {
                        p.GetValue(
                            &windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
                        )
                    })
                    .ok()
                    .and_then(|v| windows::core::BSTR::try_from(&v).ok())
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| id.clone());
                result.push(Endpoint { id, name, capture });
            }
        }
        Ok(result)
    }
}

use windows::core::{HRESULT, Interface, PROPVARIANT, implement};
struct Activated(usize);
impl Drop for Activated {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                drop(IAudioClient::from_raw(self.0 as *mut _));
            }
        }
    }
}
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion {
    tx: std::sync::mpsc::SyncSender<std::result::Result<Activated, HRESULT>>,
    _params: Box<AUDIOCLIENT_ACTIVATION_PARAMS>,
}
impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        operation: Option<&IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        unsafe {
            let result = (|| -> windows::core::Result<IAudioClient> {
                let mut status = HRESULT(0);
                let mut object = None;
                operation
                    .ok_or_else(|| {
                        windows::core::Error::from_hresult(HRESULT(0x80004003u32 as i32))
                    })?
                    .GetActivateResult(&mut status, &mut object)?;
                status.ok()?;
                object
                    .ok_or_else(|| {
                        windows::core::Error::from_hresult(HRESULT(0x80004003u32 as i32))
                    })?
                    .cast()
            })()
            .map(|client| Activated(client.into_raw() as usize))
            .map_err(|e| e.code());
            let _ = self.tx.send(result);
            Ok(())
        }
    }
}
fn process_client(pid: u32, cancel: &AtomicBool) -> Result<IAudioClient> {
    unsafe {
        let mut params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        });
        let mut raw: windows::core::imp::PROPVARIANT = std::mem::zeroed();
        raw.Anonymous.Anonymous.vt = 65; // VT_BLOB
        raw.Anonymous.Anonymous.Anonymous.blob = windows::core::imp::BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: (&mut *params as *mut AUDIOCLIENT_ACTIVATION_PARAMS).cast(),
        };
        // Blob is owned by the callback, not PROPVARIANT. Never free its Box via PropVariantClear.
        let variant = std::mem::ManuallyDrop::new(PROPVARIANT::from_raw(raw));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let completion: IActivateAudioInterfaceCompletionHandler = Completion {
            tx,
            _params: params,
        }
        .into();
        let _operation=ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,&IAudioClient::IID,Some(&*variant),&completion).context("Application audio capture requires Windows build 20348 or newer (Windows 11 recommended)")?;
        let started = std::time::Instant::now();
        loop {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("Audio activation cancelled")
            }
            match rx.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(Ok(mut raw)) => {
                    let client = IAudioClient::from_raw(raw.0 as *mut _);
                    raw.0 = 0;
                    return Ok(client);
                }
                Ok(Err(hr)) => return Err(windows::core::Error::from_hresult(hr).into()),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("Application audio activation disconnected")
                }
                Err(_) => ensure!(
                    started.elapsed() < std::time::Duration::from_secs(10),
                    "Application audio activation timed out"
                ),
            }
        }
    }
}
