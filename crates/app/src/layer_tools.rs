impl Studio {
 fn update_widget_geometry(&mut self){
  let Some(canvas)=self.project.canvases.get(self.canvas)else{return};let Some(scene)=self.project.scene(canvas.scene)else{return};let selection=self.selections.get(&scene.id).cloned().unwrap_or_else(||self.layer.iter().copied().collect());
  let dimensions:Vec<_>=selection.iter().filter_map(|&i|scene.layers.get(i)).map(|l|{let w=l.rect[2]*canvas.width as f32/(1.-l.crop[0]-l.crop[2]);let h=l.rect[3]*canvas.height as f32/(1.-l.crop[1]-l.crop[3]);let scale=960./w.max(h).max(1.);(l.source,(w*scale).round().clamp(96.,960.)as u32,(h*scale).round().clamp(96.,960.)as u32)}).collect();
  for(id,w,h)in dimensions{if let Some(Source{kind:SourceKind::Widget(s),..})=self.project.sources.iter_mut().find(|s|s.id==id){s.layout_width=w;s.layout_height=h;}}
 }

 fn select_layer(&mut self,scene:Id,index:usize,add:bool){
  let Some(s)=self.project.scene(scene)else{return};let group=s.layers[index].group;let members:Vec<_>=s.layers.iter().enumerate().filter(|(i,l)|*i==index||(group!=0&&l.group==group)).map(|(i,_)|i).collect();let selected=self.selections.entry(scene).or_default();
  if !add{*selected=members;}else{for i in members{if selected.contains(&i){selected.retain(|n|*n!=i);}else{selected.push(i);}}}self.layer=selected.last().copied();
 }
 /// The small toolbar under a preview: Arrange when several things are selected, and one ⋯ menu for the rest.
 fn canvas_layout_tools(&mut self,ui:&mut egui::Ui){
  let canvas=self.project.canvases[self.canvas].clone();let Some(si)=self.project.scenes.iter().position(|s|s.id==canvas.scene)else{return};
  let mut selected=self.selections.get(&canvas.scene).cloned().unwrap_or_default();selected.retain(|i|*i<self.project.scenes[si].layers.len());
  let mut locked=self.project.preferences.locked_canvases.contains(&canvas.id);
  let destinations:Vec<_>=self.project.canvases.iter().enumerate().filter(|(_,c)|c.id!=canvas.id).map(|(i,c)|(i,c.name.clone())).collect();
  ui.horizontal_wrapped(|ui|{
   ui.menu_button("⋯",|ui|{
    if ui.checkbox(&mut locked,"Lock this video (nothing can be moved)").changed(){self.project.preferences.locked_canvases.retain(|id|*id!=canvas.id);if locked{self.project.preferences.locked_canvases.push(canvas.id);}self.changed();}
    let before=self.project.preferences.snapping;
    let settings=&mut self.project.preferences.snapping;
    ui.checkbox(&mut settings.enabled,"Snap to edges and other items");
    if settings.enabled{ui.indent("snap",|ui|{ui.checkbox(&mut settings.canvas_edges,"Video edges");ui.checkbox(&mut settings.canvas_center,"Video center");ui.checkbox(&mut settings.sources,"Other items");ui.add(egui::DragValue::new(&mut settings.distance).range(2.0..=24.0).suffix(" px snap distance"));});}
    if self.project.preferences.snapping!=before{self.changed();}
    if !selected.is_empty()&&!locked{for(index,name)in destinations.clone(){ui.menu_button(format!("Copy selection to {name}"),|ui|{if ui.button("Share source settings").clicked(){self.copy_layers_to(si,&selected,index,false);ui.close();}if ui.button("Make independent copies").clicked(){self.copy_layers_to(si,&selected,index,true);ui.close();}});}}
    if self.advanced(){ui.checkbox(&mut self.animate,"Animate test input");}
    ui.separator();
    note(ui,"Drag things on the video to move them; drag a corner to resize. Shift-click to select more, or drag across empty space. Hold Ctrl to skip snapping.");
   }).response.on_hover_text("Lock, snapping and selection options");
   if locked{note(ui,"🔒 Locked");}
   let editable:Vec<_>=selected.iter().copied().filter(|&i|!self.project.scenes[si].layers[i].locked).collect();
   if editable.len()>1&&!locked{ui.menu_button(format!("Arrange {} items",editable.len()),|ui|{
    if ui.button("Group").clicked(){let group=self.project.scenes[si].layers.iter().map(|l|l.group).max().unwrap_or(0)+1;for &i in &editable{self.project.scenes[si].layers[i].group=group;}self.changed();ui.close();}
    if ui.button("Ungroup").clicked(){for &i in &editable{self.project.scenes[si].layers[i].group=0;}self.changed();ui.close();}
    ui.separator();
    for(name,axis,factor)in[("Align left",0,0.),("Align horizontal centers",0,0.5),("Align right",0,1.),("Align top",1,0.),("Align vertical centers",1,0.5),("Align bottom",1,1.)]{if ui.button(name).clicked(){if let Some(b)=auroracast_core::layout::bounds(&self.project.scenes[si],&editable){for &i in &editable{let l=&mut self.project.scenes[si].layers[i];l.rect[axis]=b[axis]+b[axis+2]*factor-l.rect[axis+2]*factor;}self.changed();}ui.close();}}
    for(name,axis)in[("Equal horizontal gaps",0),("Equal vertical gaps",1)]{if ui.add_enabled(editable.len()>=3,egui::Button::new(name)).clicked(){auroracast_core::layout::distribute(&mut self.project.scenes[si],&editable,axis);self.changed();ui.close();}}
   });}
  });
 }
 fn copy_layers_to(&mut self,scene:usize,indices:&[usize],canvas:usize,independent:bool){
  let destination=self.project.canvases[canvas].scene;let originals:Vec<_>=indices.iter().filter_map(|&i|self.project.scenes[scene].layers.get(i).cloned()).collect();let mut copies=Vec::new();let mut cloned=std::collections::BTreeMap::new();let mut groups=std::collections::BTreeMap::new();let mut next_group=self.project.scene(destination).unwrap().layers.iter().map(|l|l.group).max().unwrap_or(0)+1;
  for mut layer in originals{if independent{let old=layer.source;let id=if let Some(&id)=cloned.get(&old){id}else{let mut source=self.project.source(old).unwrap().clone();let id=self.project.id();source.id=id;source.name=format!("{} copy",source.name);self.project.sources.push(source);if let Some(value)=self.project.preferences.browser_sources.get(&old).cloned(){self.project.preferences.browser_sources.insert(id,value);}if let Some(value)=self.project.preferences.chroma_keys.get(&old).cloned(){self.project.preferences.chroma_keys.insert(id,value);}if let Some(mut audio)=self.project.audio.iter().find(|c|c.media_source==Some(old)).cloned(){audio.id=self.project.id();audio.media_source=Some(id);audio.name=format!("{} copy",audio.name);self.project.audio.push(audio);}
cloned.insert(old,id);id};layer.source=id;}
   if layer.group!=0{layer.group=*groups.entry(layer.group).or_insert_with(||{let n=next_group;next_group+=1;n});}copies.push(layer);
  }
  self.project.scenes.iter_mut().find(|s|s.id==destination).unwrap().layers.extend(copies);self.changed();self.message="Selection copied to the destination canvas's current scene".into();
 }
 fn layer_rename_window(&mut self,ctx:&egui::Context){
  let Some((id,mut name))=self.layer_rename.clone()else{return};
  if self.project.source(id).is_none(){self.layer_rename=None;return;}
  let open=true;let mut save=false;let mut cancel=false;
  egui::Modal::new(egui::Id::new("rename_layer_dialog")).frame(solid_window(ctx)).show(ctx,|ui|{
   ui.set_width(340.);ui.heading("Rename layer");ui.label("Name");let field=ui.add(egui::TextEdit::singleline(&mut name).char_limit(100).desired_width(320.));
   if !ctx.memory(|m|m.has_focus(field.id))&&!ctx.memory(|m|m.focused().is_some()){field.request_focus();}
   ui.label("Updates this source's name in every scene. Placement stays the same.");
   let valid=!name.trim().is_empty();
   ui.horizontal(|ui|{save=ui.add_enabled(valid,egui::Button::new("Save")).clicked()||(valid&&field.lost_focus()&&ui.input(|i|i.key_pressed(egui::Key::Enter)));cancel=ui.button("Cancel").clicked();});
  });
  if ctx.input(|i|i.key_pressed(egui::Key::Escape)){cancel=true;}
  if save{
   if let Some(source)=self.project.sources.iter_mut().find(|s|s.id==id){let old=std::mem::replace(&mut source.name,name.trim().to_owned());for channel in self.project.audio.iter_mut().filter(|c|c.media_source==Some(id)&&c.name==old){channel.name=name.trim().to_owned();}}
   self.changed();self.layer_rename=None;
  }else if !open||cancel{self.layer_rename=None;}else{self.layer_rename=Some((id,name));}
 }
 fn layer_tools_windows(&mut self,ctx:&egui::Context){
  self.layer_rename_window(ctx);
  self.layer_properties_window(ctx);
  if let Some((scene,i))=self.pending_layer_delete.take(){if let Some(s)=self.project.scenes.iter_mut().find(|s|s.id==scene){if s.layers.get(i).is_some_and(|l|!l.locked){let source=s.layers.remove(i).source;self.selections.remove(&scene);self.layer=None;self.crop_layer=None;self.changed();let name=self.project.source(source).map(|s|s.name.clone()).unwrap_or_default();self.deleted(format!("Removed {name} from this scene"));}}}
  let Some((scene,index))=self.crop_layer else{return};let Some(si)=self.project.scenes.iter().position(|s|s.id==scene)else{self.crop_layer=None;return};let Some(layer)=self.project.scenes[si].layers.get(index).cloned()else{self.crop_layer=None;return};
  let size=self.captures.reader().latest().into_iter().find(|(id,_)|*id==layer.source).and_then(|(_,frame)|frame).map(|f|[f.width as f32,f.height as f32]).or_else(||match &self.project.source(layer.source)?.kind{SourceKind::Image{path}=>image::image_dimensions(path).ok().map(|(w,h)|[w as f32,h as f32]),_=>None});
  let mut crop=layer.crop;let mut open=true;let mut changed=false;
  egui::Window::new("Crop source").open(&mut open).default_pos(egui::pos2(30.,120.)).default_width(280.).show(ctx,|ui|{
   ui.label("Drag the gold canvas handles, or enter crop values here. Cropping is specific to this layer.");
   ui.add_enabled_ui(!layer.locked,|ui|{for(i,label)in["Left","Top","Right","Bottom"].iter().enumerate(){let unit=size.map_or(100.,|s|s[i%2]);let mut value=crop[i]*unit;changed|=ui.add(egui::DragValue::new(&mut value).range(0.0..=unit*0.99).prefix(format!("{label}: ")).suffix(if size.is_some(){" px"}else{" %"})).changed();crop[i]=value/unit;}if ui.button("Reset crop").clicked(){crop=[0.;4];changed=true;}});
  });if changed{auroracast_core::layout::set_crop(&mut self.project.scenes[si].layers[index],crop);self.changed();}if !open{self.crop_layer=None;}
 }
}
