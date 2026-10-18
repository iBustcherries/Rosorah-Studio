impl Studio {
 fn utility_windows(&mut self,ctx:&egui::Context){
  if let Some(rx)=&self.tools.job&&let Ok(result)=rx.try_recv(){self.tools.job=None;match result{
   Ok(crate::utilities::Finished::External(preview))=>{self.tools.external_preview=Some(preview);self.tools.status="Review the import below before creating a new project.".into();},
   Ok(crate::utilities::Finished::Saved(path))=>{self.tools.status=format!("Saved {}",path.display());if path.extension().is_some_and(|e|e=="zip"){self.tools.file=path.display().to_string();}else{self.tools.recording_file=path.display().to_string();}},
   Ok(crate::utilities::Finished::Imported(project,path))=>{self.save();if !self.dirty{self.project_path=path;self.adopt_project(project);self.tools.status="Project imported. Local media paths were restored. Connect accounts on this computer separately.".into();}else{self.tools.status=format!("Imported to {}. Current project could not be saved; it has been left open.",path.display());}},
   Ok(crate::utilities::Finished::Located(id,path))=>{if let Some(source)=self.project.sources.iter_mut().find(|s|s.id==id){match &mut source.kind{SourceKind::Image{path:p}=>*p=path.display().to_string(),SourceKind::VideoFile(v)=>v.path=path.display().to_string(),_=>{}}self.changed();}self.tools.status="Media location updated".into();},
   Err(error)=>self.tools.status=error,
  }}
  if self.tools.test_pending{if let Some(r)=&self.live_recording{if r.view().phase==crate::recording::Phase::Recording{self.tools.test_pending=false;self.tools.test_until=Some(Instant::now()+Duration::from_secs(10));}}else{self.tools.test_pending=false;}}
  if self.tools.test_until.is_some_and(|t|Instant::now()>=t){if let Some(r)=&self.live_recording{r.stop();self.tools.recording_file=r.view().path.display().to_string();}self.tools.test_until=None;self.tools.status="Local test complete. Play the recording to check picture, sound and sync.".into();}
  self.compatibility_window(ctx);self.project_recording_window(ctx);self.monitoring_window(ctx);self.performance_window(ctx);self.setup_window(ctx);
 }
 fn project_recording_window(&mut self,ctx:&egui::Context){if !self.tools.open{return}let mut open=true;egui::Window::new("Project & recording tools").open(&mut open).default_width(650.).default_height(550.).show(ctx,|ui|{egui::ScrollArea::vertical().show(ui,|ui|{
  ui.heading("Portable project");ui.label("Bundle scenes, layouts, automations and local image/video/audio files. Account sign-ins and stream keys stay on this computer. Browser-source URLs remain in the project.");
  ui.label("OBS and Firebot imports are in Settings > Advanced.");ui.separator();
  ui.label("File path (optional; leave blank to use the file picker)");ui.text_edit_singleline(&mut self.tools.file);
  ui.label("Import destination folder (optional)");ui.text_edit_singleline(&mut self.tools.folder);
  ui.add_enabled_ui(self.tools.job.is_none()&&self.live_stream.is_none()&&self.live_recording.is_none(),|ui|{ui.horizontal(|ui|{
   if ui.button("Export project bundle…").clicked(){let project=self.project.clone();let path=self.tools.file.clone();self.tools.spawn(move||{let path=if path.trim().is_empty(){crate::utilities::picker(true,false,"Export Rosorah project bundle")?}else{path.into()};crate::utilities::bundle(Some(project),path,None)});}
   if ui.button("Import project bundle…").clicked(){let path=self.tools.file.clone();let parent=self.tools.folder.clone();self.tools.spawn(move||{let path=if path.trim().is_empty(){crate::utilities::picker(false,false,"Open Rosorah project bundle")?}else{path.into()};let folder=if parent.trim().is_empty(){crate::utilities::picker(false,true,"Choose where to unpack the project")?}else{parent.into()};crate::utilities::bundle(None,path,Some(folder))});}
  });});
  let missing:Vec<_>=self.project.sources.iter().filter_map(|s|{let path=match &s.kind{SourceKind::Image{path}=>path.as_str(),SourceKind::VideoFile(v)=>v.path.as_str(),_=>return None};(!std::path::Path::new(path).is_file()).then_some((s.id,s.name.clone()))}).collect();
  for(id,name)in missing{ui.horizontal(|ui|{ui.colored_label(Color32::LIGHT_RED,format!("Missing: {name}"));if ui.add_enabled(self.tools.job.is_none(),egui::Button::new("Locate…")).clicked(){self.tools.spawn(move||Ok(crate::utilities::Finished::Located(id,crate::utilities::picker(false,false,"Locate missing media")?)));}});}
  ui.separator();ui.heading("Recording recovery & conversion");ui.label("MKV recordings are written in short clusters. Recovery rebuilds the container from readable data; frames never written cannot be recovered. The original file is retained.");
  ui.label("Recording path (optional; leave blank to choose a file)");ui.text_edit_singleline(&mut self.tools.recording_file);
  for path in crate::utilities::incomplete(std::path::Path::new(&self.record_folder)){ui.horizontal(|ui|{ui.label(path.file_name().unwrap_or_default().to_string_lossy());if ui.add_enabled(self.tools.job.is_none()&&self.live_recording.is_none()&&self.live_stream.is_none(),egui::Button::new("Recover MKV")).clicked(){self.tools.spawn(move||crate::utilities::remux(path,false));}});}
  ui.add_enabled_ui(self.tools.job.is_none()&&self.live_recording.is_none()&&self.live_stream.is_none(),|ui|{ui.horizontal(|ui|{if ui.button("Convert recording to MP4…").clicked(){let path=self.tools.recording_file.clone();self.tools.spawn(move||{let path=if path.is_empty(){crate::utilities::picker(false,false,"Choose recording to convert")?}else{path.into()};crate::utilities::remux(path,true)});}if ui.button("Recover another MKV…").clicked(){self.tools.spawn(move||crate::utilities::remux(crate::utilities::picker(false,false,"Choose incomplete recording")?,false));}});});
  ui.small("Conversion copies the audio/video without re-encoding. It requires MP4-compatible codecs such as H.264 + AAC; incompatible recordings keep their original format.");
  if self.tools.job.is_some(){ui.spinner();ctx.request_repaint_after(Duration::from_millis(100));}ui.label(&self.tools.status);
 });});self.tools.open=open;}
 fn monitoring_window(&mut self,ctx:&egui::Context){if !self.tools.audio{return}let mut open=true;let mut changed=false;let view=self.audio.view();egui::Window::new("Monitoring & sync").open(&mut open).default_width(620.).show(ctx,|ui|{
  ui.label("Output is what your stream/recording receives. Monitoring is what you hear in your headphones.");let before=self.project.preferences.monitor_device.clone();egui::ComboBox::from_id_salt("headphone-output").selected_text(view.devices.iter().find(|d|d.target==before).map_or("Choose headphones / monitoring off",|d|d.label.as_str())).show_ui(ui,|ui|{ui.selectable_value(&mut self.project.preferences.monitor_device,String::new(),"Monitoring off");for d in view.devices.iter().filter(|d|d.kind=="Audio/Sink"){ui.selectable_value(&mut self.project.preferences.monitor_device,d.target.clone(),&d.label);}});changed|=before!=self.project.preferences.monitor_device;
  if ui.button("Refresh audio devices").clicked(){self.audio.refresh();}ui.small(&view.monitor_status);ui.small("Avoid monitoring desktop audio back into the same captured speakers.");
  for channel in &mut self.project.audio{ui.push_id(channel.id,|ui|{ui.separator();ui.strong(&channel.name);let old=(channel.monitor,channel.monitor_only);let mut mode=if channel.monitor_only{2}else if channel.monitor{1}else{0};egui::ComboBox::from_id_salt("route-mode").selected_text(["Output only","Monitor & output","Monitor only"][mode]).show_ui(ui,|ui|{for(i,label)in["Output only","Monitor & output","Monitor only"].iter().enumerate(){ui.selectable_value(&mut mode,i,*label);}});channel.monitor=mode!=0;channel.monitor_only=mode==2;changed|=old!=(channel.monitor,channel.monitor_only);changed|=ui.add(egui::DragValue::new(&mut channel.delay_ms).range(0..=2000).suffix(" ms delay")).changed();});}
  ui.label("Increase delay when the audio arrives before the picture. Delay affects both monitoring and output; mute/volume still follow the current scene.");
 });self.tools.audio=open;if changed{self.audio.monitor_device(&self.project.preferences.monitor_device);self.changed();}}
 fn performance_window(&mut self,ctx:&egui::Context){if !self.tools.performance{return}let mut open=true;egui::Window::new("Performance details").open(&mut open).default_width(600.).show(ctx,|ui|{
  for(name,busy)in &self.resources.gpu{ui.label(format!("{name}: {busy}% whole-device GPU busy"));}if self.resources.gpu.is_empty(){ui.label("GPU busy counter is not exposed by this driver.");}
  let mut views=Vec::new();if let Some(r)=&self.live_recording{views.push(("Recording",r.view()));}if let Some(s)=&self.live_stream{views.push(("Stream encoder",s.view()));}
  if views.is_empty(){ui.label("Start a recording or stream to see output timing.");}
  for(name,v)in views{ui.separator();ui.strong(name);ui.label(format!("Last render: {:.1} ms · missed render deadlines: {}",v.render_ms,v.missed_frames));ui.label(format!("Encoder backlog: {} ms · discarded queued frames: {}",v.encoder_lag_ms,v.queue_drops));ui.small(&v.encoder);}
  if let Some(stream)=&self.live_stream{ui.separator();for d in stream.destinations(){ui.label(format!("{}: {} · connection attempts {}",d.name,d.phase.label(),d.attempts));if !d.detail.is_empty(){ui.small(&d.detail);}}}
  ui.separator();ui.label("Missed render deadlines indicate compositor/scheduling pressure. Growing encoder backlog indicates output processing is falling behind. Destination reconnects indicate an output/network problem. GPU busy includes other apps and games.");
 });self.tools.performance=open;}
}

impl Studio{
 /// Preview and import an OBS or Firebot backup into a new Rosorah project.
 pub(crate) fn external_import_ui(&mut self,ui:&mut egui::Ui){
  ui.label("Scenes, sources, text, filters, audio and transitions from OBS; events, commands, counters and effects from Firebot. Everything that can't be converted is listed before you import.");
  ui.label("Replacement media folder (optional; matches unique filenames)");ui.text_edit_singleline(&mut self.tools.media_folder);
  let media=self.tools.media_folder.clone();let idle=self.tools.job.is_none();
  ui.horizontal_wrapped(|ui|{
   for (label,found) in [("Find OBS on this computer",crate::external_import::obs_folder()),("Find Firebot on this computer",crate::external_import::firebot_folder())]{
    if let Some(folder)=found{if ui.add_enabled(idle,egui::Button::new(label)).on_hover_text(folder.display().to_string()).clicked(){let media=media.clone();self.tools.spawn(move||crate::external_import::read(folder,media));}}
   }
   if ui.add_enabled(idle,egui::Button::new("Choose a file or ZIP…")).on_hover_text("OBS scene collection JSON, profile basic.ini or exported ZIP; Firebot backup ZIP or JSON").clicked(){let media=media.clone();self.tools.spawn(move||crate::external_import::read(crate::utilities::picker(false,false,"Choose OBS or Firebot export")?,media));}
   if ui.add_enabled(idle,egui::Button::new("Choose a folder…")).on_hover_text("An OBS settings folder (obs-studio) or Firebot folder (Firebot/v5)").clicked(){let media=media.clone();self.tools.spawn(move||crate::external_import::read(crate::utilities::picker(false,true,"Choose the OBS or Firebot folder")?,media));}
  });
  if crate::external_import::obs_folder().is_none()&&crate::external_import::firebot_folder().is_none(){ui.small("OBS and Firebot weren't found in their usual folders; choose a file or folder instead.");}
  if let Some(preview)=&self.tools.external_preview{ui.label(format!("{} scenes · {} sources · {} automations",preview.project.scenes.len(),preview.project.sources.len(),preview.project.rules.len()));egui::ScrollArea::vertical().id_salt("import_report").max_height(180.).show(ui,|ui|{for w in &preview.warnings{ui.label(w);}for p in &preview.missing{ui.colored_label(egui::Color32::YELLOW,format!("Missing media: {p}"));}});
   if ui.add_enabled(self.tools.job.is_none()&&self.live_stream.is_none()&&self.live_recording.is_none(),egui::Button::new("Create new project from preview…")).clicked(){let project=preview.project.clone();self.tools.spawn(move||{let path=crate::desktop::pick(crate::desktop::Pick::Save,"Save imported Rosorah project","Imported-project.json")?.ok_or_else(||anyhow::anyhow!("Cancelled"))?;crate::external_import::save(project,path)});}
  }
 if self.tools.job.is_some(){ui.spinner();ui.ctx().request_repaint_after(Duration::from_millis(100));}if !self.tools.status.is_empty(){ui.label(&self.tools.status);}
 }
 fn compatibility_window(&mut self,ctx:&egui::Context){if !self.tools.compatibility{return}let mut open=true;let r=crate::desktop::report();egui::Window::new("Desktop compatibility").open(&mut open).default_width(580.).show(ctx,|ui|{
 ui.heading(format!("{} · {:?}",r.desktop,r.session));ui.label("These checks describe this session's interfaces, not certification of the desktop or GPU.");
 if r.checking{ui.spinner();ui.label("Checking desktop services…");ctx.request_repaint_after(Duration::from_millis(100));}
 ui.separator();ui.strong("Capture");if let Some(types)=r.sources{ui.label(format!("Desktop portal: {}{}",if types&1!=0{"screens "}else{""},if types&2!=0{"windows"}else{""}));}else{ui.label("Desktop screen-sharing portal unavailable.");}
 if let Some(modes)=r.cursors{ui.label(format!("Pointer modes: {}{}",if modes&1!=0{"hidden "}else{""},if modes&2!=0{"embedded"}else{""}));}
 #[cfg(windows)] ui.label("Windows DXGI monitor capture, WASAPI audio, native file dialogs and Credential Manager. Window/game capture is not yet available.");
 if !r.portal_error.is_empty(){ui.small(&r.portal_error);}#[cfg(not(windows))]
 if r.session==crate::desktop::Session::X11{ui.label("X11 fallback: screens directly; window selection through xwininfo. Requires the GStreamer ximagesrc plugin.");}else{ui.label("Wayland screen/window capture needs a portal backend for this desktop. A backend that offers screens only cannot provide individual windows.");}
 ui.label("Automatic game capture uses renderer hooks independently of desktop sharing. Game/runtime compatibility still applies.");ui.separator();
 ui.label(if r.file_chooser{"File dialogs: desktop portal available"}else{"File dialogs: portal unavailable; desktop dialog fallback will be tried"});ui.label(if r.wallet{"Account storage: Secret Service is available or activatable"}else{"Account storage: no Secret Service found; sign-in may be session-only until a wallet is enabled"});
 ui.label("Music discovery: desktop MPRIS interface. Automation shortcuts: Rosorah must have focus.");ui.separator();ui.strong("Local tools");for(name,present)in &r.commands{ui.label(format!("{name}: {}",if *present{"installed"}else{"not found"}));}ui.small("PipeWire audio tools are required for microphone, desktop and game-audio routing. FFmpeg and your GPU drivers determine available encoding; this check does not change either.");
 if ui.add_enabled(!r.checking,egui::Button::new("Refresh checks")).clicked(){crate::desktop::refresh();}
 });self.tools.compatibility=open;}
}
