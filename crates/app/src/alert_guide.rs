//! Beginner flow. Creates ordinary editable rules, with no second execution engine.
use auroracast_core::{*,widgets::{Settings,Kind,AlertLayout}};
use eframe::egui;
use std::sync::mpsc;
pub struct Guide {
 pub open:bool, step:usize, name:String, event:EventKind, platform:Option<Platform>,
 text:String, chat:String, chat_enabled:bool, targets:Vec<Platform>, media:String,sound:String,
 gain:f32, monitor:bool, seconds:u32, canvas:usize, layout:AlertLayout, phrase:String,reward:String,
 picker:Option<(bool,mpsc::Receiver<Option<std::path::PathBuf>>)>, notice:String,
 /// Built-in samples offered as quick picks, supplied by the editor.
 pub samples:Vec<crate::alert_samples::Sample>,
}
impl Default for Guide {fn default()->Self{Self{open:false,step:0,name:"New alert".into(),event:EventKind::Follow,platform:None,text:"Thanks, {user}!".into(),chat:String::new(),chat_enabled:false,targets:vec![],media:String::new(),sound:String::new(),gain:1.,monitor:true,seconds:6,canvas:0,layout:AlertLayout::Below,phrase:String::new(),reward:String::new(),picker:None,notice:String::new(),samples:Vec::new()}}}
impl Guide {
 pub fn design_preview(step:usize)->Self{Self{open:true,step:step.min(2),name:"Donation celebration".into(),event:EventKind::Donation,text:"Thank you, {user}, for {amount} {currency}!".into(),..Default::default()}}
 pub fn for_reward(&mut self,id:&str,title:&str){*self=Self::default();self.open=true;self.event=EventKind::Redemption;self.platform=Some(Platform::Twitch);self.reward=id.into();self.name=title.into();}
 pub fn ui(&mut self,ctx:&egui::Context,p:&mut Project,connected:&[Platform])->Option<(Id,bool)>{
  if !self.open{return None;} if let Some((sound,rx))=&self.picker&& let Ok(path)=rx.try_recv(){if let Some(path)=path {if *sound{self.sound=path.to_string_lossy().into()}else{self.media=path.to_string_lossy().into()}}self.picker=None;}
  let mut open=true;let mut result=None;
  egui::Window::new("Create alert or automation").open(&mut open).default_width(570.).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).resizable(true).show(ctx,|ui|{
   ui.horizontal(|ui|{for (i,label) in ["1  When","2  Do","3  Preview & finish"].iter().enumerate(){ui.selectable_value(&mut self.step,i,*label);}});ui.separator();
   egui::ScrollArea::vertical().max_height(550.).show(ui,|ui|{
   match self.step {
    0=>{
     ui.heading("What should start this alert?");
     ui.horizontal_wrapped(|ui|{for (label,event,text) in [("Follow",EventKind::Follow,"Thanks for following, {user}!"),("Donation",EventKind::Donation,"Thank you, {user}, for {amount} {currency}!"),("First message",EventKind::FirstMessage,"Welcome, {user}!"),("Raid",EventKind::Raid,"Welcome, {user}, and {viewers} raiders!"),("Channel points",EventKind::Redemption,"{user} redeemed a reward!"),("Chat phrase",EventKind::ChatMessage,"{user}: {message}")]{if ui.button(label).clicked(){self.event=event;self.name=label.into();self.text=text.into();}}});
     ui.label("Name");ui.text_edit_singleline(&mut self.name);
     egui::ComboBox::from_id_salt("guide_event").selected_text(self.event.label()).show_ui(ui,|ui|{for e in EventKind::ALL{ui.selectable_value(&mut self.event,e,e.label());}});
     egui::ComboBox::from_id_salt("guide_platform").selected_text(self.platform.map(|p|p.name()).unwrap_or("Any connected platform")).show_ui(ui,|ui|{ui.selectable_value(&mut self.platform,None,"Any connected platform");for &p in connected{ui.selectable_value(&mut self.platform,Some(p),p.name());}ui.selectable_value(&mut self.platform,Some(Platform::Donations),"Donation services");});
     if connected.is_empty(){ui.label("You can prepare an alert now. Connect accounts in Settings to receive events.");}
     if matches!(self.event,EventKind::ChatMessage|EventKind::ChatCommand){ui.label("Word, phrase, or command");ui.text_edit_singleline(&mut self.phrase);}
     if self.event==EventKind::Redemption{ui.label("Reward ID (leave empty for any reward)");ui.text_edit_singleline(&mut self.reward);ui.small("Use Twitch rewards > Link automation to select a reward without copying an ID.");}
    },
    1=>{
     ui.heading("What should viewers see and hear?");
     for sound in [false,true]{ui.label(if sound{"Optional sound file"}else{"Optional video / image / GIF"});ui.horizontal(|ui|{ui.text_edit_singleline(if sound{&mut self.sound}else{&mut self.media});if ui.button(if sound{"Browse sound…"}else{"Browse media…"}).clicked(){let(tx,rx)=mpsc::channel();self.picker=Some((sound,rx));std::thread::spawn(move||{let _=tx.send(crate::desktop::pick(crate::desktop::Pick::Open,"Choose alert media","").ok().flatten());});}});}
     let dropped=ctx.input(|i|i.raw.dropped_files.clone());for f in dropped {if let Some(path)=f.path{let ext=path.extension().and_then(|s|s.to_str()).unwrap_or("").to_ascii_lowercase();if ["mp3","wav","ogg","flac","aac","m4a"].contains(&ext.as_str()){self.sound=path.to_string_lossy().into()}else{self.media=path.to_string_lossy().into()}}}
     let samples=self.samples.clone();if !samples.is_empty(){ui.horizontal_wrapped(|ui|{ui.small("Built-in:");for sample in samples{if ui.small_button(sample.name).on_hover_text(if sample.sound{"Use as the sound"}else{"Use as the picture / video"}).clicked(){let path=sample.path.to_string_lossy().into_owned();if sample.sound{self.sound=path}else{self.media=path}}}});}
     ui.small("Drop files here. Playback formats depend on installed GStreamer codecs.");
     ui.add(egui::Slider::new(&mut self.gain,0.0..=1.5).text("Sound volume"));
     ui.checkbox(&mut self.monitor,"Hear this alert on my speakers / headphones");
     ui.small("Stream audio stays enabled. If Desktop audio captures the same output, use a separate monitoring device to avoid hearing the alert twice on stream.");
     ui.label("Message on screen");ui.add(egui::TextEdit::multiline(&mut self.text).desired_rows(3));ui.small("Use {user}, {amount}, {currency}, {message}, {count}, {platform}.");
     ui.horizontal(|ui|{for layout in [AlertLayout::Above,AlertLayout::Below,AlertLayout::Over]{ui.selectable_value(&mut self.layout,layout,layout.label());}});
     ui.add(egui::Slider::new(&mut self.seconds,1..=60).text("Alert duration (seconds)"));
     egui::ComboBox::from_id_salt("guide_canvas").selected_text(p.canvases.get(self.canvas).map(|c|c.name.as_str()).unwrap_or("Choose canvas")).show_ui(ui,|ui|{for(i,c)in p.canvases.iter().enumerate(){ui.selectable_value(&mut self.canvas,i,&c.name);}});
     ui.small("All alerts use one shared Alerts box. Adding an alert reuses it; another scene gets the same box with its own placement.");
     ui.separator();ui.checkbox(&mut self.chat_enabled,"Also send a chat message");if self.chat_enabled{ui.text_edit_multiline(&mut self.chat);ui.small("No boxes checked: reply on the triggering platform. Donation services have no chat; select Twitch or YouTube explicitly.");for platform in [Platform::Twitch,Platform::YouTube]{let mut yes=self.targets.contains(&platform);if ui.checkbox(&mut yes,platform.name()).changed(){self.targets.retain(|p|*p!=platform);if yes{self.targets.push(platform);}}}}
    },
    _=>{
     ui.heading(&self.name);ui.label(format!("When: {} · {}",self.event.label(),self.platform.map(|p|p.name()).unwrap_or("Any connected platform")));
     ui.label(format!("Display for {} seconds · {}",self.seconds,self.layout.label()));ui.group(|ui|{ui.label(self.text.replace("{user}","Sample viewer").replace("{amount}","5.00").replace("{currency}","USD"));});
     if self.chat_enabled {ui.label(format!("Chat: {}",self.chat));}
     ui.small("Test creates a disabled rule and plays the preview while offline. It never posts chat or changes goals. Enable the rule after reviewing it.");
     ui.horizontal(|ui|{if ui.button("Create & test").clicked(){match self.create(p,false){Ok(id)=>result=Some((id,true)),Err(e)=>self.notice=e}}if ui.button("Create & enable").clicked(){match self.create(p,true){Ok(id)=>result=Some((id,false)),Err(e)=>self.notice=e}}});
    }
   }
   if !self.notice.is_empty(){ui.colored_label(egui::Color32::LIGHT_RED,&self.notice);}
   });ui.separator();ui.horizontal(|ui|{if self.step>0&&ui.button("Back").clicked(){self.step-=1;}if self.step<2&&ui.button("Next").clicked(){self.step+=1;}});
  });self.open=open;if result.is_some(){*self=Self::default();}result
 }
 fn create(&self,p:&mut Project,enabled:bool)->Result<Id,String>{
  if self.name.trim().is_empty(){return Err("Give this alert a name.".into());}
  for path in [&self.media,&self.sound]{if !path.is_empty()&&!std::path::Path::new(path).is_file(){return Err(format!("Locate the missing file: {path}"));}}
  if self.chat_enabled&&(self.chat.trim().is_empty()||self.chat.chars().count()>500){return Err("Chat messages must contain 1–500 characters.".into());}
  let canvas=p.canvases.get(self.canvas).ok_or("Choose a canvas")?.clone();
  let existing=p.sources.iter().find(|s|matches!(&s.kind,SourceKind::Widget(w) if w.kind==Kind::AlertBox)).map(|s|s.id);
  let box_id=if let Some(id)=existing {
    if let Some(source)=p.sources.iter_mut().find(|s|s.id==id)&&let SourceKind::Widget(settings)=&mut source.kind{settings.alert_layout=self.layout;}
    id
  }else{
    let id=p.id();// 16:9 by default, so 16:9 media fills it and 4:3 media fits top to bottom without stretching.
    let mut settings=Settings{kind:Kind::AlertBox,label:"Alerts".into(),alert_layout:self.layout,automation_only:true,layout_width:960,layout_height:540,..Default::default()};settings.platforms.push(Platform::Donations);
    p.sources.push(Source{id,name:"Alerts".into(),kind:SourceKind::Widget(settings)});
    if let Some(s)=p.scenes.iter_mut().find(|s|s.id==canvas.scene){s.layers.push(Layer{ frame: Default::default(), look: Default::default(),source:id,rect:alert_box_rect(&canvas),visible:true,locked:false,opacity:1.,crop:[0.;4],group:0});}
    id
  };
  if let Some(scene)=p.scenes.iter_mut().find(|s|s.id==canvas.scene) {
   if let Some(layer)=scene.layers.iter_mut().find(|l|l.source==box_id){layer.visible=true;}else{scene.layers.push(Layer{ frame: Default::default(), look: Default::default(),source:box_id,rect:alert_box_rect(&canvas),visible:true,locked:false,opacity:1.,crop:[0.;4],group:0});}
  }
  let mut add=|path:&str,sound:bool|->Option<Id>{if path.is_empty(){return None;}let id=p.id();p.sources.push(Source{id,name:format!("{} · {}",self.name,if sound{"Sound"}else{"Media"}),kind:SourceKind::VideoFile(VideoSettings{path:path.into(),gain:self.gain,audio:sound||self.sound.is_empty(),looped:false,automation_only:true,duration_secs:self.seconds,canvases:vec![canvas.id],..Default::default()})});
   let audio_id=p.id();p.audio.push(AudioChannel{id:audio_id,name:format!("{} · {}",self.name,if sound{"Sound"}else{"Media"}),gain_db:0.,muted:false,tracks:[true,false,false,false,false,false],delay_ms:0,monitor:self.monitor,monitor_only:false,noise_suppression:Default::default(),media_source:Some(id)});
   if sound&& let Some(s)=p.scenes.iter_mut().find(|s|s.id==canvas.scene){s.layers.push(Layer{ frame: Default::default(), look: Default::default(),source:id,rect:[0.,0.,0.001,0.001],visible:true,locked:true,opacity:0.,crop:[0.;4],group:0});}
   Some(id)};
  let media=add(&self.media,false);let sound=add(&self.sound,true);
  if self.monitor&&(media.is_some()||sound.is_some())&&p.preferences.monitor_device.is_empty(){p.preferences.monitor_device=crate::audio::DEFAULT_MONITOR.into();}
  let mut actions=vec![Action::DisplayAlert{source:box_id,template:self.text.clone(),media,seconds:self.seconds,subject:Default::default()}];if let Some(source)=sound{actions.push(Action::PlaySound{source});}if self.chat_enabled{actions.push(Action::SendChat{text:self.chat.clone(),platforms:self.targets.clone(),sender:Default::default()});}
  let id=p.id();p.rules.push(Rule{id,name:self.name.clone(),enabled,platform:self.platform,event:self.event,command:if self.event==EventKind::ChatCommand{self.phrase.clone()}else{String::new()},cooldown_ms:3000,actions,conditions:alert_filters::Conditions{phrase:if self.event==EventKind::ChatMessage{self.phrase.clone()}else{String::new()},reward_id:self.reward.clone(),..Default::default()},options:Default::default()});Ok(id)
 }
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn guided_rule_is_disabled_for_test_and_has_targeted_box(){let mut p=Project::default();let mut g=Guide::default();g.chat_enabled=true;g.chat="Thanks {user}!".into();let id=g.create(&mut p,false).unwrap();p.validate().unwrap();let r=p.rules.iter().find(|r|r.id==id).unwrap();assert!(!r.enabled);assert!(matches!(r.actions[0],Action::DisplayAlert{..}));let preview=crate::automations::preview_actions(r.actions.clone(),true);assert!(!preview.iter().any(|a|matches!(a,Action::SendChat{..})));let replay=crate::automations::replay_actions(r.actions.clone());assert!(!replay.iter().any(|a|matches!(a,Action::SendChat{..})));}
 #[test]fn missing_media_leaves_project_untouched(){let mut p=Project::default();let before=serde_json::to_string(&p).unwrap();let mut g=Guide::default();g.media="/not/a/real/file.mp4".into();assert!(g.create(&mut p,true).is_err());assert_eq!(serde_json::to_string(&p).unwrap(),before);}
}

#[cfg(test)]mod monitoring_tests {
 use super::*;
 #[test]fn guided_media_is_audible_without_disabling_stream_audio(){
  let path=std::env::temp_dir().join(format!("rosorah-alert-audio-{}.wav",std::process::id()));std::fs::write(&path,b"test").unwrap();
  let result=||{let mut p=Project::default();let mut g=Guide::default();g.media=path.to_string_lossy().into();let id=g.create(&mut p,false).unwrap();
   let media=match p.rules.iter().find(|r|r.id==id).unwrap().actions[0]{Action::DisplayAlert{media:Some(id),..}=>id,_=>panic!("missing media")};
   let channel=p.audio.iter().find(|c|c.media_source==Some(media)).unwrap();assert!(channel.monitor);assert!(channel.tracks[0]);assert!(!channel.monitor_only);assert_eq!(p.preferences.monitor_device,crate::audio::DEFAULT_MONITOR);
   assert!(!p.audio_for_scene(p.canvases[0].scene).iter().find(|c|c.media_source==Some(media)).unwrap().muted);
   p.preferences.monitor_device="user-chosen-headphones".into();g.create(&mut p,false).unwrap();assert_eq!(p.preferences.monitor_device,"user-chosen-headphones");
   let mut p=Project::default();g.monitor=false;g.create(&mut p,false).unwrap();assert!(p.preferences.monitor_device.is_empty());assert!(p.audio.iter().filter(|c|c.media_source.is_some()).all(|c|!c.monitor));
  };result();let _=std::fs::remove_file(path);
 }
}

#[cfg(test)]mod shared_box_tests {
 use super::*;
 #[test]fn many_alerts_and_scenes_share_one_box_including_manual_boxes(){
  let mut p=Project::default();let box_id=p.id();p.sources.push(Source{id:box_id,name:"My alert box".into(),kind:SourceKind::Widget(Settings{kind:Kind::AlertBox,..Default::default()})});
  let mut g=Guide::default();for event in [EventKind::Follow,EventKind::Raid,EventKind::Donation]{g.event=event;g.create(&mut p,true).unwrap();}
  if p.canvases.len()>1{g.canvas=1;g.create(&mut p,true).unwrap();}
  assert_eq!(p.sources.iter().filter(|s|matches!(&s.kind,SourceKind::Widget(w) if w.kind==Kind::AlertBox)).count(),1);
  for r in &p.rules{for a in &r.actions{if let Action::DisplayAlert{source,..}=a{assert_eq!(*source,box_id);}}}
  for scene in &p.scenes{assert!(scene.layers.iter().filter(|l|l.source==box_id).count()<=1);}
  p.validate().unwrap();
 }
}

/// A 16:9 alert box centred near the top of `canvas`, half its width (narrower on a tall canvas
/// so it still fits).
fn alert_box_rect(canvas: &auroracast_core::Canvas) -> [f32; 4] {
    let ratio = canvas.width.max(1) as f32 / canvas.height.max(1) as f32;
    let mut w = 0.5;
    let mut h = w * ratio * 9. / 16.;
    if h > 0.6 {
        w *= 0.6 / h;
        h = 0.6;
    }
    [(1. - w) / 2., 0.08, w, h]
}
#[cfg(test)]
mod alert_box_tests {
    #[test]
    fn alert_box_is_16_by_9_on_wide_and_tall_canvases() {
        for (cw, ch) in [(1920u32, 1080u32), (1080, 1920)] {
            let canvas = auroracast_core::Canvas { width: cw, height: ch, ..auroracast_core::Project::default().canvases[0].clone() };
            let [x, y, w, h] = super::alert_box_rect(&canvas);
            let shape = (w * cw as f32) / (h * ch as f32);
            assert!((shape - 16. / 9.).abs() < 0.01, "{cw}x{ch}: {shape}");
            assert!(x >= 0. && y >= 0. && x + w <= 1. && y + h <= 1.);
        }
    }
}
