// Included in studio.rs so widget editing uses the same project transaction path.
#[derive(Default)]
struct WidgetEditor {
    id: Option<Id>,
    sample: bool,
    texture: Option<egui::TextureHandle>,
    key: u64,
    preset_name: String,
    copied: Option<widgets::Style>,
    last: Option<Instant>,
}
/// A new widget's starting look for its kind.
fn widget_settings(kind: widgets::Kind) -> widgets::Settings {
    let mut settings = widgets::Settings {
        kind,
        label: kind.label().into(),
        ..Default::default()
    };
    settings.apply_theme(widgets::Theme::Material);
    if kind == widgets::Kind::AlertBox {
        settings.platforms.push(Platform::Donations);
    }
    if kind == widgets::Kind::Chat {
        settings.apply_theme(widgets::Theme::Unified);
        settings.font_size = 28;
        settings.label.clear();
    }
    if kind == widgets::Kind::Text {
        settings.background = [0, 0, 0, 0];
        settings.foreground = [255, 255, 255, 255];
        settings.font_size = 0;
        settings.outline_width = 3;
        settings.content = "Your text here".into();
    }
    if kind == widgets::Kind::NowPlaying {
        settings.font_size = 42;
        settings.background = [18, 22, 30, 238];
    }
    settings
}
impl Studio {
    fn restore_layer_proportions(&mut self, scene: usize, layer: usize) {
        let id = self.project.scenes[scene].layers[layer].source;
        let size = self
            .captures
            .reader()
            .latest()
            .into_iter()
            .find(|(source, _)| *source == id)
            .and_then(|(_, f)| f)
            .map(|f| (f.width, f.height))
            .or_else(|| {
                self.project.source(id).and_then(|s| match &s.kind {
                    SourceKind::Widget(s) => Some((s.layout_width, s.layout_height)),
                    SourceKind::Image { path } => image::image_dimensions(path).ok(),
                    _ => None,
                })
            });
        if let Some((w, h)) = size.filter(|(w, h)| *w > 0 && *h > 0) {
            let canvas = &self.project.canvases[self.canvas];
            let l = &mut self.project.scenes[scene].layers[layer];
            l.rect[3] =
                l.rect[2] * canvas.width as f32 / canvas.height as f32 * h as f32 / w as f32 * (1.-l.crop[1]-l.crop[3])/(1.-l.crop[0]-l.crop[2]);
            self.changed();
        } else {
            self.message = "Start this source first so its dimensions can be detected".into();
        }
    }

    /// Points alerts whose box is gone at `box_id`; alerts given a box of their own keep it.
    fn route_visual_alerts(&mut self,box_id:Id){
        let boxes:Vec<Id>=self.project.sources.iter().filter(|s|matches!(&s.kind,SourceKind::Widget(w) if w.kind==widgets::Kind::AlertBox)).map(|s|s.id).collect();
        for rule in &mut self.project.rules {for action in &mut rule.actions {if let auroracast_core::Action::DisplayAlert{source,..}=action && !boxes.contains(source){*source=box_id;}}}
    }
    /// Another alert box for one alert step: added to this scene, and the step shows in it.
    pub(crate) fn new_alert_box(&mut self, rule: Id, step: usize) {
        let count = self.project.sources.iter().filter(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == widgets::Kind::AlertBox)).count();
        let id = self.create_widget(widgets::Kind::AlertBox);
        if let Some(s) = self.project.sources.iter_mut().find(|s| s.id == id) {
            s.name = format!("Alerts {}", count + 1);
        }
        if let Some(auroracast_core::Action::DisplayAlert { source, .. }) = self.project.rules.iter_mut().find(|r| r.id == rule).and_then(|r| r.actions.get_mut(step)) {
            *source = id;
        }
        self.message = format!("Added Alerts {}: move and size it on the canvas", count + 1);
        self.changed();
    }
    fn add_widget(&mut self, kind: widgets::Kind) {
        let scene = self.project.canvases[self.canvas].scene;
        if kind == widgets::Kind::AlertBox {
            if let Some(id) = self.project.sources.iter().find(|s| matches!(&s.kind, SourceKind::Widget(w) if w.kind == widgets::Kind::AlertBox)).map(|s|s.id) {
                if let Some(scene) = self.project.scenes.iter_mut().find(|s|s.id==scene) {
                    let index = if let Some(i)=scene.layers.iter().position(|l|l.source==id) {scene.layers[i].visible=true;i} else {
                        scene.layers.push(Layer{ frame: Default::default(), look: Default::default(),source:id,rect:[0.15,0.1,0.7,0.35],visible:true,locked:false,opacity:1.,crop:[0.;4],group:0});scene.layers.len()-1
                    };
                    self.layer=Some(index);self.selections.insert(scene.id,vec![index]);
                }
                self.route_visual_alerts(id);
                self.widget_editor.id=Some(id);self.widget_editor.sample=true;self.widget_editor.key=0;
                self.message="Using the shared Alerts box".into();self.changed();return;
            }
        }
        let id = self.create_widget(kind);
        if kind==widgets::Kind::AlertBox {self.route_visual_alerts(id);}
        self.widget_editor.id = Some(id);
        self.widget_editor.sample = true;
        self.widget_editor.key = 0;
        self.changed();
    }
    /// Adds a new widget of this kind to the current scene and selects its layer.
    fn create_widget(&mut self, kind: widgets::Kind) -> Id {
        let scene = self.project.canvases[self.canvas].scene;
        let id = self.project.id();
        let settings = widget_settings(kind);
        let canvas = &self.project.canvases[self.canvas];
        let ratio=if kind==widgets::Kind::Sports{if settings.sports.layout==widgets::ScoreLayout::Ticker{8.}else{5.}}else{2.4};
        let height = (0.8 * canvas.width as f32 / canvas.height as f32 / ratio).min(0.8);
        self.project.sources.push(Source {
            id,
            name: if kind==widgets::Kind::AlertBox {"Alerts".into()}else{kind.label().into()},
            kind: SourceKind::Widget(settings),
        });
        if let Some(scene) = self.project.scenes.iter_mut().find(|s| s.id == scene) {
            scene.layers.push(Layer { crop: [0.;4], group: 0, frame: Default::default(), look: Default::default(),
                source: id,
                rect: [0.1, 0.1, 0.8, height],
                visible: true,
                locked: false,
                opacity: 1.,
            });
            self.layer = Some(scene.layers.len() - 1);
        }
        self.selections.insert(scene,vec![self.layer.unwrap_or(0)]);
        id
    }
    fn widget_editor_window(&mut self, ctx: &egui::Context) {
        let mut editor = std::mem::take(&mut self.widget_editor);
        let Some(id) = editor.id else {
            self.widget_editor = editor;
            return;
        };
        if !self
            .project
            .source(id)
            .is_some_and(|s| matches!(s.kind, SourceKind::Widget(_)))
        {
            editor.id = None;
            self.widget_editor = editor;
            return;
        }
        let mut open = true;
        egui::Window::new("Widget studio").id(egui::Id::new("widget-studio")).open(&mut open).default_size([1020.,650.]).min_width(760.).max_height((ctx.content_rect().height()-90.).max(300.)).anchor(egui::Align2::CENTER_CENTER,[0.,0.]).show(ctx,|ui|{
   let name=self.project.source(id).unwrap().name.clone();ui.horizontal(|ui|{ui.heading(name);ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.label(egui::RichText::new("Changes apply immediately · Ctrl+Z to undo").small().color(muted()));});});ui.separator();
   ui.columns(2,|cols|{
    egui::ScrollArea::vertical().id_salt("widget-controls").max_height(550.).show(&mut cols[0],|ui|{
     self.widget_controls(ui,id);
     ui.separator();ui.strong("Your styles");ui.small("Styles copy appearance only. Goals, text and connections stay intact.");
     let SourceKind::Widget(settings)=&self.project.source(id).unwrap().kind else{return};let style=widgets::Style::from_settings(settings);
     ui.horizontal(|ui|{if ui.button("Copy style").clicked(){editor.copied=Some(style.clone());}if ui.add_enabled(editor.copied.is_some(),egui::Button::new("Paste style")).clicked(){if let Some(style)=&editor.copied{self.apply_widget_style(id,style);}}});
     ui.add(egui::TextEdit::singleline(&mut editor.preset_name).hint_text("Name this style").char_limit(60));
     if ui.add_enabled(!editor.preset_name.trim().is_empty(),egui::Button::new("Save style")).clicked(){self.project.preferences.widget_styles.insert(editor.preset_name.trim().to_string(),style.clone());self.changed();}
     let presets:Vec<_>=self.project.preferences.widget_styles.iter().map(|(n,s)|(n.clone(),s.clone())).collect();
     for (name,style) in presets{ui.horizontal(|ui|{if ui.button(&name).clicked(){self.apply_widget_style(id,&style);}if ui.small_button("×").on_hover_text("Delete saved style").clicked(){self.project.preferences.widget_styles.remove(&name);self.changed();}});}
     if ui.button("Apply this style to scene widgets").clicked(){let scene=self.project.canvases[self.canvas].scene;let ids:Vec<_>=self.project.scene(scene).map(|s|s.layers.iter().map(|l|l.source).collect()).unwrap_or_default();for source in ids{if let Some(Source{kind:SourceKind::Widget(settings),..})=self.project.sources.iter_mut().find(|s|s.id==source){style.apply(settings);}}self.changed();}
    });
    let ui=&mut cols[1];ui.horizontal(|ui|{ui.strong("Preview");ui.selectable_value(&mut editor.sample,true,"Sample data");ui.selectable_value(&mut editor.sample,false,"Live");});
    ui.small(if editor.sample{"Sample preview only — no alerts, chat posts or goal changes are triggered."}else{"Uses the source frame when visible. Hidden sources show a static preview."});
    if let SourceKind::Widget(settings)=&self.project.source(id).unwrap().kind{
     use std::hash::{Hash,Hasher};let mut hash=std::collections::hash_map::DefaultHasher::new();serde_json::to_vec(settings).unwrap_or_default().hash(&mut hash);editor.sample.hash(&mut hash);let key=hash.finish();
     let refresh=key!=editor.key||editor.texture.is_none()||(!editor.sample && editor.last.is_none_or(|t|t.elapsed()>Duration::from_millis(250)));
     if refresh {
      let live=if !editor.sample{self.captures.reader().latest().into_iter().find(|(source,_)|*source==id).and_then(|(_,frame)|frame)}else{None};
      let frame=live.or_else(||crate::widgets::preview(settings,editor.sample).ok().map(std::sync::Arc::new));
      if let Some(frame)=frame{let image=egui::ColorImage::from_rgba_unmultiplied([frame.width as usize,frame.height as usize],&frame.rgba);if let Some(texture)=&mut editor.texture{texture.set(image,egui::TextureOptions::LINEAR);}else{editor.texture=Some(ctx.load_texture("widget-preview",image,egui::TextureOptions::LINEAR));}}
      editor.key=key;editor.last=Some(Instant::now());
     }
    }
    if let Some(texture)=&editor.texture{let aspect=texture.size()[0]as f32/texture.size()[1]as f32;let width=ui.available_width().min(390.*aspect);let (rect,_)=ui.allocate_exact_size(egui::vec2(width,width/aspect),egui::Sense::hover());let painter=ui.painter();painter.rect_filled(rect,12.,Color32::from_rgb(38,42,50));for y in 0..((rect.height()/16.).ceil()as i32){for x in 0..((rect.width()/16.).ceil()as i32){if(x+y)%2==0{painter.rect_filled(egui::Rect::from_min_size(rect.min+egui::vec2(x as f32*16.,y as f32*16.),egui::vec2(16.,16.)).intersect(rect),0.,Color32::from_rgb(43,47,56));}}}painter.image(texture.id(),rect,egui::Rect::from_min_max(egui::pos2(0.,0.),egui::pos2(1.,1.)),Color32::WHITE);}
    ui.add_space(16.);ui.strong("On your canvas");ui.label("Resize or move the widget in the canvas. Right-click its layer to edit it again. A shared source keeps the same settings across canvases.");
    if let Some(Source{kind:SourceKind::Widget(settings),..})=self.project.source(id){if settings.kind==widgets::Kind::NowPlaying {let (_,status)=crate::music::players();ui.separator();ui.label(status);}}
   });
  });
        if !open {
            editor.id = None;
            editor.texture = None;
        }
        self.widget_editor = editor;
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    fn apply_widget_style(&mut self, id: Id, style: &widgets::Style) {
        if let Some(Source {
            kind: SourceKind::Widget(settings),
            ..
        }) = self.project.sources.iter_mut().find(|s| s.id == id)
        {
            style.apply(settings);
            self.changed();
        }
    }
    fn undo_edit(&mut self, redo: bool) {
        let result = if redo {
            self.edit_history.redo(&self.project)
        } else {
            self.edit_history.undo(&self.project)
        };
        if let Some(project) = result {
            self.project = project;
            self.selections.clear();self.crop_layer=None;
            self.layer = None;
            self.canvas = self
                .canvas
                .min(self.project.canvases.len().saturating_sub(1));
            self.camera_drafts.clear();
            self.browser_drafts.clear();
            self.notify_changed();
            self.message = if redo { "Edit restored" } else { "Edit undone" }.into();
        }
    }
    fn project_tools(&mut self, ctx: &egui::Context) {
        if !ctx.wants_keyboard_input() {
            let undo = ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
            let redo = ctx.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                ) || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y)
            });
            if redo {
                self.undo_edit(true)
            } else if undo {
                self.undo_edit(false)
            }
        }
        if self.recovery_candidate.is_some() {
            let mut action = 0;
            egui::Modal::new(egui::Id::new("recover-unsaved")).frame(solid_window(ctx)).show(ctx,|ui|{ui.set_width(380.);ui.heading("Recover unsaved changes?");ui.label("A newer recovery copy was found. Your saved project has not been replaced.");ui.horizontal(|ui|{if ui.button("Recover changes").clicked(){action=1;}if ui.button("Keep saved project").clicked(){action=2;}});});
            if action == 1 {
                let project = self.recovery_candidate.take().unwrap();
                self.adopt_project(project);
                self.changed();
            } else if action == 2 {
                self.recovery_candidate = None;
                let _ =
                    std::fs::remove_file(crate::edit_history::recovery_path(&self.project_path));
            }
        }
        if let Some(rx) = &self.recovery_write
            && let Ok(result) = rx.try_recv()
        {
            if let Err(error) = result {
                self.message = format!("Recovery copy could not be saved: {error}");
            }
            self.recovery_write = None;
        }
        if self.dirty
            && self.recovery_candidate.is_none()
            && self.recovery_write.is_none()
            && self.last_recovery.elapsed() > Duration::from_secs(15)
        {
            self.checkpoint_widgets();
            let project = self.project.clone();
            let path = crate::edit_history::recovery_path(&self.project_path);
            let (tx, rx) = mpsc::channel();
            self.recovery_write = Some(rx);
            self.last_recovery = Instant::now();
            std::thread::spawn(move || {
                let _ = tx.send(project.save(&path).map_err(|e| e.to_string()));
            });
        }
    }
}
