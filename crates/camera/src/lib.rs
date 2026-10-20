//! Rosorah Virtual Camera for Windows 11: a Media Foundation camera source that Windows' camera
//! service (Frame Server) loads while Rosorah has the camera switched on. It reads finished NV12
//! frames from Rosorah's pipe and hands them to whichever apps opened the camera. When Rosorah
//! isn't sending, apps see black rather than a frozen picture. Written from Microsoft's
//! documentation of the interfaces; nothing else is drawn or encoded here.
#![cfg(windows)]
#![allow(non_snake_case)]
use std::{
    io::Read,
    sync::{
        Arc, Mutex, Once,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Media::{KernelStreaming::*, MediaFoundation::*},
        System::Com::*,
    },
    core::*,
};

/// The component's class id (Rosorah registers the camera with it).
pub const CLSID: GUID = GUID::from_u128(0xb6ac0fba_5752_439c_ad7b_9bbc8d093265);
/// The picture apps get. Rosorah scales its canvas to this before sending.
const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const FPS: u32 = 30;
const FRAME_BYTES: usize = (WIDTH * HEIGHT * 3 / 2) as usize;
const PIPE: &str = r"\\.\pipe\RosorahVirtualCamera";
/// "The property set isn't supported": the answer to camera controls we don't have.
const NOT_FOUND: HRESULT = HRESULT(0x8007_0492_u32 as i32);

/// The newest frame from Rosorah, shared by the reader thread and the stream.
struct Frames {
    latest: Mutex<Option<Vec<u8>>>,
    /// When the newest frame arrived (ms since `start`), to fall back to black when Rosorah stops.
    arrived: AtomicU64,
    start: Instant,
    stop: AtomicBool,
}
impl Frames {
    fn new() -> Arc<Self> {
        let frames = Arc::new(Self { latest: Mutex::new(None), arrived: AtomicU64::new(0), start: Instant::now(), stop: AtomicBool::new(false) });
        let f = frames.clone();
        std::thread::spawn(move || read_frames(&f));
        frames
    }
    /// Copies the newest frame (or black) into `out`.
    fn fill(&self, out: &mut [u8]) {
        let fresh = self.start.elapsed().as_millis() as u64 <= self.arrived.load(Ordering::Relaxed) + 2000;
        match self.latest.lock().unwrap().as_ref() {
            Some(frame) if fresh => out.copy_from_slice(frame),
            _ => {
                let (y, uv) = out.split_at_mut((WIDTH * HEIGHT) as usize);
                y.fill(16);
                uv.fill(128);
            }
        }
    }
}
/// Connects to Rosorah's pipe whenever it's there and keeps the newest frame.
fn read_frames(frames: &Frames) {
    let mut buffer = vec![0u8; FRAME_BYTES];
    while !frames.stop.load(Ordering::Relaxed) {
        let Ok(mut pipe) = std::fs::File::open(PIPE) else {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };
        while !frames.stop.load(Ordering::Relaxed) && pipe.read_exact(&mut buffer).is_ok() {
            let old = frames.latest.lock().unwrap().replace(std::mem::take(&mut buffer));
            buffer = old.unwrap_or_else(|| vec![0u8; FRAME_BYTES]);
            frames.arrived.store(frames.start.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
    }
}

fn media_type() -> Result<IMFMediaType> {
    unsafe {
        let t = MFCreateMediaType()?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, ((WIDTH as u64) << 32) | HEIGHT as u64)?;
        t.SetUINT64(&MF_MT_FRAME_RATE, ((FPS as u64) << 32) | 1)?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1 << 32) | 1)?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        t.SetUINT32(&MF_MT_DEFAULT_STRIDE, WIDTH)?;
        t.SetUINT32(&MF_MT_SAMPLE_SIZE, FRAME_BYTES as u32)?;
        // Rosorah converts with HD colours in video range; say so, so apps don't guess.
        t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32)?;
        t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32)?;
        Ok(t)
    }
}
/// Marks attributes as describing a shared colour camera stream.
fn describe_stream(a: &IMFAttributes) -> Result<()> {
    unsafe {
        a.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
        a.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
        a.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
        a.SetUINT32(&MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MFFrameSourceTypes_Color.0 as u32)?;
        Ok(())
    }
}
fn attributes() -> Result<IMFAttributes> {
    let mut a = None;
    unsafe { MFCreateAttributes(&mut a, 4)? };
    a.ok_or_else(|| E_OUTOFMEMORY.into())
}
fn queue_simple(queue: &IMFMediaEventQueue, event: MF_EVENT_TYPE) -> Result<()> {
    unsafe { queue.QueueEventParamVar(event.0 as u32, &GUID::zeroed(), S_OK, std::ptr::null()) }
}

#[implement(IMFMediaStream2, IKsControl)]
struct Stream {
    queue: IMFMediaEventQueue,
    descriptor: IMFStreamDescriptor,
    attributes: IMFAttributes,
    source: Mutex<Option<IMFMediaSource>>,
    state: Mutex<MF_STREAM_STATE>,
    frames: Arc<Frames>,
}
impl Stream {
    fn set_running(&self, running: bool) -> Result<()> {
        *self.state.lock().unwrap() = if running { MF_STREAM_STATE_RUNNING } else { MF_STREAM_STATE_STOPPED };
        queue_simple(&self.queue, if running { MEStreamStarted } else { MEStreamStopped })
    }
    fn shutdown(&self) {
        self.source.lock().unwrap().take();
        unsafe {
            let _ = self.queue.Shutdown();
        }
    }
    fn sample(&self) -> Result<IMFSample> {
        unsafe {
            let buffer = MFCreateMemoryBuffer(FRAME_BYTES as u32)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            self.frames.fill(std::slice::from_raw_parts_mut(data, FRAME_BYTES));
            buffer.Unlock()?;
            buffer.SetCurrentLength(FRAME_BYTES as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(MFGetSystemTime())?;
            sample.SetSampleDuration(10_000_000 / FPS as i64)?;
            Ok(sample)
        }
    }
}
impl IMFMediaEventGenerator_Impl for Stream_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        unsafe { self.queue.GetEvent(flags.0) }
    }
    fn BeginGetEvent(&self, callback: Option<&IMFAsyncCallback>, state: Option<&IUnknown>) -> Result<()> {
        unsafe { self.queue.BeginGetEvent(callback, state) }
    }
    fn EndGetEvent(&self, result: Option<&IMFAsyncResult>) -> Result<IMFMediaEvent> {
        unsafe { self.queue.EndGetEvent(result) }
    }
    fn QueueEvent(&self, met: u32, extended: *const GUID, status: HRESULT, value: *const PROPVARIANT) -> Result<()> {
        unsafe { self.queue.QueueEventParamVar(met, extended, status, value) }
    }
}
impl IMFMediaStream_Impl for Stream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        self.source.lock().unwrap().clone().ok_or_else(|| MF_E_SHUTDOWN.into())
    }
    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        Ok(self.descriptor.clone())
    }
    fn RequestSample(&self, token: Option<&IUnknown>) -> Result<()> {
        if self.source.lock().unwrap().is_none() {
            return Err(MF_E_SHUTDOWN.into());
        }
        if *self.state.lock().unwrap() != MF_STREAM_STATE_RUNNING {
            return Err(MF_E_INVALIDREQUEST.into());
        }
        let sample = self.sample()?;
        unsafe {
            if let Some(token) = token {
                sample.SetUnknown(&MFSampleExtension_Token, token)?;
            }
            self.queue.QueueEventParamUnk(MEMediaSample.0 as u32, &GUID::zeroed(), S_OK, &sample)
        }
    }
}
impl IMFMediaStream2_Impl for Stream_Impl {
    fn SetStreamState(&self, value: MF_STREAM_STATE) -> Result<()> {
        match value {
            MF_STREAM_STATE_RUNNING => self.set_running(true),
            MF_STREAM_STATE_STOPPED => self.set_running(false),
            _ => Err(MF_E_INVALID_STATE_TRANSITION.into()),
        }
    }
    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        Ok(*self.state.lock().unwrap())
    }
}
impl IKsControl_Impl for Stream_Impl {
    fn KsProperty(&self, _: *const KSIDENTIFIER, _: u32, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(NOT_FOUND.into())
    }
    fn KsMethod(&self, _: *const KSIDENTIFIER, _: u32, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(NOT_FOUND.into())
    }
    fn KsEvent(&self, _: *const KSIDENTIFIER, _: u32, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(NOT_FOUND.into())
    }
}

#[implement(IMFMediaSourceEx, IMFGetService, IKsControl)]
struct Source {
    queue: IMFMediaEventQueue,
    attributes: IMFAttributes,
    descriptor: IMFPresentationDescriptor,
    stream: Mutex<Option<ComObject<Stream>>>,
    announced: AtomicBool,
    frames: Arc<Frames>,
}
fn new_source() -> Result<IMFMediaSource> {
    unsafe {
        let kind = media_type()?;
        let descriptor = MFCreateStreamDescriptor(0, &[Some(kind.clone())])?;
        descriptor.GetMediaTypeHandler()?.SetCurrentMediaType(&kind)?;
        describe_stream(&descriptor.cast()?)?;
        let presentation = MFCreatePresentationDescriptor(Some(&[Some(descriptor.clone())]))?;
        presentation.SelectStream(0)?;
        let stream_attributes = attributes()?;
        describe_stream(&stream_attributes)?;
        let frames = Frames::new();
        let stream = ComObject::new(Stream {
            queue: MFCreateEventQueue()?,
            descriptor,
            attributes: stream_attributes,
            source: Mutex::new(None),
            state: Mutex::new(MF_STREAM_STATE_STOPPED),
            frames: frames.clone(),
        });
        let source = ComObject::new(Source {
            queue: MFCreateEventQueue()?,
            attributes: attributes()?,
            descriptor: presentation,
            stream: Mutex::new(Some(stream.clone())),
            announced: AtomicBool::new(false),
            frames,
        });
        let interface: IMFMediaSource = source.to_interface::<IMFMediaSourceEx>().into();
        *stream.source.lock().unwrap() = Some(interface.clone());
        Ok(interface)
    }
}
impl Source {
    fn stream(&self) -> Result<ComObject<Stream>> {
        self.stream.lock().unwrap().clone().ok_or_else(|| MF_E_SHUTDOWN.into())
    }
}
impl IMFMediaEventGenerator_Impl for Source_Impl {
    fn GetEvent(&self, flags: MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS) -> Result<IMFMediaEvent> {
        unsafe { self.queue.GetEvent(flags.0) }
    }
    fn BeginGetEvent(&self, callback: Option<&IMFAsyncCallback>, state: Option<&IUnknown>) -> Result<()> {
        unsafe { self.queue.BeginGetEvent(callback, state) }
    }
    fn EndGetEvent(&self, result: Option<&IMFAsyncResult>) -> Result<IMFMediaEvent> {
        unsafe { self.queue.EndGetEvent(result) }
    }
    fn QueueEvent(&self, met: u32, extended: *const GUID, status: HRESULT, value: *const PROPVARIANT) -> Result<()> {
        unsafe { self.queue.QueueEventParamVar(met, extended, status, value) }
    }
}
impl IMFMediaSource_Impl for Source_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        self.stream()?;
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }
    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        self.stream()?;
        unsafe { self.descriptor.Clone() }
    }
    fn Start(&self, _: Option<&IMFPresentationDescriptor>, _: *const GUID, _: *const PROPVARIANT) -> Result<()> {
        let stream = self.stream()?;
        // The first start introduces the stream; later ones say it's the same one again.
        let event = if self.announced.swap(true, Ordering::Relaxed) { MEUpdatedStream } else { MENewStream };
        let unknown: IUnknown = stream.to_interface();
        unsafe { self.queue.QueueEventParamUnk(event.0 as u32, &GUID::zeroed(), S_OK, &unknown)? };
        stream.set_running(true)?;
        queue_simple(&self.queue, MESourceStarted)
    }
    fn Stop(&self) -> Result<()> {
        self.stream()?.set_running(false)?;
        queue_simple(&self.queue, MESourceStopped)
    }
    fn Pause(&self) -> Result<()> {
        Err(MF_E_INVALID_STATE_TRANSITION.into())
    }
    fn Shutdown(&self) -> Result<()> {
        self.frames.stop.store(true, Ordering::Relaxed);
        if let Some(stream) = self.stream.lock().unwrap().take() {
            stream.shutdown();
        }
        unsafe { self.queue.Shutdown() }
    }
}
impl IMFMediaSourceEx_Impl for Source_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        Ok(self.attributes.clone())
    }
    fn GetStreamAttributes(&self, id: u32) -> Result<IMFAttributes> {
        if id != 0 {
            return Err(MF_E_INVALIDSTREAMNUMBER.into());
        }
        Ok(self.stream()?.attributes.clone())
    }
    fn SetD3DManager(&self, _: Option<&IUnknown>) -> Result<()> {
        // Frames are plain memory; no graphics device is needed.
        Ok(())
    }
}
impl IMFGetService_Impl for Source_Impl {
    fn GetService(&self, _: *const GUID, _: *const GUID, _: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}
impl IKsControl_Impl for Source_Impl {
    fn KsProperty(&self, _: *const KSIDENTIFIER, _: u32, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(NOT_FOUND.into())
    }
    fn KsMethod(&self, _: *const KSIDENTIFIER, _: u32, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(NOT_FOUND.into())
    }
    fn KsEvent(&self, _: *const KSIDENTIFIER, _: u32, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> Result<()> {
        Err(NOT_FOUND.into())
    }
}

/// What Windows creates first: it makes the source when the camera is actually opened.
#[implement(IMFActivate)]
struct Activate {
    attributes: IMFAttributes,
    source: Mutex<Option<IMFMediaSource>>,
}
impl IMFActivate_Impl for Activate_Impl {
    fn ActivateObject(&self, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        static STARTUP: Once = Once::new();
        STARTUP.call_once(|| unsafe {
            let _ = MFStartup(MF_VERSION, MFSTARTUP_LITE);
        });
        let mut slot = self.source.lock().unwrap();
        if slot.is_none() {
            *slot = Some(new_source()?);
        }
        unsafe { slot.as_ref().unwrap().query(riid, ppv).ok() }
    }
    fn ShutdownObject(&self) -> Result<()> {
        if let Some(source) = self.source.lock().unwrap().take() {
            unsafe {
                let _ = source.Shutdown();
            }
        }
        Ok(())
    }
    fn DetachObject(&self) -> Result<()> {
        self.source.lock().unwrap().take();
        Ok(())
    }
}
/// The activate object's settings are kept in a plain attribute store.
impl IMFAttributes_Impl for Activate_Impl {
    fn GetItem(&self, key: *const GUID, value: *mut PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.GetItem(key, Some(value)) }
    }
    fn GetItemType(&self, key: *const GUID) -> Result<MF_ATTRIBUTE_TYPE> {
        unsafe { self.attributes.GetItemType(key) }
    }
    fn CompareItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<BOOL> {
        unsafe { self.attributes.CompareItem(key, value) }
    }
    fn Compare(&self, theirs: Option<&IMFAttributes>, kind: MF_ATTRIBUTES_MATCH_TYPE) -> Result<BOOL> {
        unsafe { self.attributes.Compare(theirs, kind) }
    }
    fn GetUINT32(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.GetUINT32(key) }
    }
    fn GetUINT64(&self, key: *const GUID) -> Result<u64> {
        unsafe { self.attributes.GetUINT64(key) }
    }
    fn GetDouble(&self, key: *const GUID) -> Result<f64> {
        unsafe { self.attributes.GetDouble(key) }
    }
    fn GetGUID(&self, key: *const GUID) -> Result<GUID> {
        unsafe { self.attributes.GetGUID(key) }
    }
    fn GetStringLength(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.GetStringLength(key) }
    }
    fn GetString(&self, key: *const GUID, value: PWSTR, size: u32, length: *mut u32) -> Result<()> {
        unsafe { (Interface::vtable(&self.attributes).GetString)(Interface::as_raw(&self.attributes), key, value, size, length).ok() }
    }
    fn GetAllocatedString(&self, key: *const GUID, value: *mut PWSTR, length: *mut u32) -> Result<()> {
        unsafe { self.attributes.GetAllocatedString(key, value, length) }
    }
    fn GetBlobSize(&self, key: *const GUID) -> Result<u32> {
        unsafe { self.attributes.GetBlobSize(key) }
    }
    fn GetBlob(&self, key: *const GUID, buffer: *mut u8, size: u32, written: *mut u32) -> Result<()> {
        unsafe { (Interface::vtable(&self.attributes).GetBlob)(Interface::as_raw(&self.attributes), key, buffer, size, written).ok() }
    }
    fn GetAllocatedBlob(&self, key: *const GUID, buffer: *mut *mut u8, size: *mut u32) -> Result<()> {
        unsafe { self.attributes.GetAllocatedBlob(key, buffer, size) }
    }
    fn GetUnknown(&self, key: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        unsafe { (Interface::vtable(&self.attributes).GetUnknown)(Interface::as_raw(&self.attributes), key, riid, ppv).ok() }
    }
    fn SetItem(&self, key: *const GUID, value: *const PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.SetItem(key, value) }
    }
    fn DeleteItem(&self, key: *const GUID) -> Result<()> {
        unsafe { self.attributes.DeleteItem(key) }
    }
    fn DeleteAllItems(&self) -> Result<()> {
        unsafe { self.attributes.DeleteAllItems() }
    }
    fn SetUINT32(&self, key: *const GUID, value: u32) -> Result<()> {
        unsafe { self.attributes.SetUINT32(key, value) }
    }
    fn SetUINT64(&self, key: *const GUID, value: u64) -> Result<()> {
        unsafe { self.attributes.SetUINT64(key, value) }
    }
    fn SetDouble(&self, key: *const GUID, value: f64) -> Result<()> {
        unsafe { self.attributes.SetDouble(key, value) }
    }
    fn SetGUID(&self, key: *const GUID, value: *const GUID) -> Result<()> {
        unsafe { self.attributes.SetGUID(key, value) }
    }
    fn SetString(&self, key: *const GUID, value: &PCWSTR) -> Result<()> {
        unsafe { self.attributes.SetString(key, *value) }
    }
    fn SetBlob(&self, key: *const GUID, buffer: *const u8, size: u32) -> Result<()> {
        unsafe { (Interface::vtable(&self.attributes).SetBlob)(Interface::as_raw(&self.attributes), key, buffer, size).ok() }
    }
    fn SetUnknown(&self, key: *const GUID, value: Option<&IUnknown>) -> Result<()> {
        unsafe { self.attributes.SetUnknown(key, value) }
    }
    fn LockStore(&self) -> Result<()> {
        unsafe { self.attributes.LockStore() }
    }
    fn UnlockStore(&self) -> Result<()> {
        unsafe { self.attributes.UnlockStore() }
    }
    fn GetCount(&self) -> Result<u32> {
        unsafe { self.attributes.GetCount() }
    }
    fn GetItemByIndex(&self, index: u32, key: *mut GUID, value: *mut PROPVARIANT) -> Result<()> {
        unsafe { self.attributes.GetItemByIndex(index, key, Some(value)) }
    }
    fn CopyAllItems(&self, destination: Option<&IMFAttributes>) -> Result<()> {
        unsafe { self.attributes.CopyAllItems(destination) }
    }
}

#[implement(IClassFactory)]
struct Factory;
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Option<&IUnknown>, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let activate: IMFActivate = Activate { attributes: attributes()?, source: Mutex::new(None) }.into();
        unsafe { activate.query(riid, ppv).ok() }
    }
    fn LockServer(&self, _: BOOL) -> Result<()> {
        Ok(())
    }
}

/// COM's entry point: hands out the factory for Rosorah's camera.
///
/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> HRESULT {
    if ppv.is_null() || clsid.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
        if *clsid != CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory.into();
        factory.query(riid, ppv)
    }
}
/// The component stays loaded while the camera service runs (its reader thread may be waiting).
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
