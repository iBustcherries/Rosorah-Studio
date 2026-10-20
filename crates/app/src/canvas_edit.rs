//! Canvas-space transforms. UI handles never enter the compositor or recording.
use auroracast_core::{Id, Scene, SnapSettings};
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};

#[derive(Clone)]
pub struct Drag {
    owner: egui::Id, scene: Id, rect:[f32;4], pointer:Pos2, corner:Option<usize>,
    originals:Vec<(usize,auroracast_core::Layer)>, marquee:bool, prior:Vec<usize>,
}

fn transform(r: [f32; 4], delta: Vec2, corner: Option<usize>, aspect: bool) -> [f32; 4] {
    let [x, y, w, h] = r;
    let Some(corner) = corner else {
        return [
            (x + delta.x).clamp(-w + 0.01, 0.99),
            (y + delta.y).clamp(-h + 0.01, 0.99),
            w,
            h,
        ];
    };
    let left = corner == 0 || corner == 3 || corner==7;
    let top = corner < 2 || corner==4;
    let mut nw = (w + if left { -delta.x } else { delta.x }).clamp(0.005, 4.);
    let mut nh = (h + if top { -delta.y } else { delta.y }).clamp(0.005, 4.);
    if corner==4 || corner==6 {nw=w;}
    if corner==5 || corner==7 {nh=h;}
    if aspect && corner >= 4 {
        let moved = [if left { x + w - nw } else { x }, if top { y + h - nh } else { y }, nw, nh];
        return keep_edge_aspect(moved, r, corner);
    }
    if aspect && corner<4 {
        let sx = nw / w.max(0.005);
        let sy = nh / h.max(0.005);
        let factor = if (sx - 1.).abs() >= (sy - 1.).abs() {
            sx
        } else {
            sy
        };
        let upper = (4. / w).min(4. / h);
        let factor = factor.clamp((0.005 / w).max(0.005 / h).min(upper), upper);
        nw = w * factor;
        nh = h * factor;
    }
    [
        if left { x + w - nw } else { x },
        if top { y + h - nh } else { y },
        nw,
        nh,
    ]
}

/// An edge handle with aspect lock scales the whole layer: the dragged axis sets the factor,
/// the opposite edge stays put and the other axis grows evenly around its center.
fn keep_edge_aspect(r: [f32; 4], original: [f32; 4], edge: usize) -> [f32; 4] {
    let axis = if edge == 4 || edge == 6 { 1 } else { 0 };
    let other = 1 - axis;
    let factor = (r[axis + 2] / original[axis + 2].max(0.005))
        .clamp(0.005 / original[other + 2].max(0.005), 4. / original[other + 2].max(0.005));
    let size = original[other + 2] * factor;
    let mut out = r;
    out[other + 2] = size;
    out[other] = original[other] + (original[other + 2] - size) / 2.;
    out
}

fn screen_rect(r: [f32; 4], canvas: Rect) -> Rect {
    Rect::from_min_size(
        canvas.min + Vec2::new(r[0] * canvas.width(), r[1] * canvas.height()),
        Vec2::new(r[2] * canvas.width(), r[3] * canvas.height()),
    )
}

#[cfg(test)]
pub fn edit(ui:&mut egui::Ui,canvas:Rect,scene:&mut Scene,selected:&mut Option<usize>,drag:&mut Option<Drag>,aspect:bool,snapping:SnapSettings)->bool {
 let mut list=selected.iter().copied().collect();edit_multi(ui,canvas,scene,selected,&mut list,drag,aspect,snapping,false,false).0
}

/// The front-most visible layer under `point` on a canvas drawn in `canvas`.
pub fn layer_at(canvas:Rect,scene:&Scene,point:Pos2)->Option<usize>{
 let at=(point-canvas.min)/canvas.size();
 scene.layers.iter().enumerate().rev().find(|(_,l)|l.visible&&at.x>=l.rect[0]&&at.y>=l.rect[1]&&at.x<=l.rect[0]+l.rect[2]&&at.y<=l.rect[1]+l.rect[3]).map(|(i,_)|i)
}
pub fn edit_multi(ui:&mut egui::Ui,canvas:Rect,scene:&mut Scene,selected:&mut Option<usize>,selection:&mut Vec<usize>,drag:&mut Option<Drag>,aspect:bool,snapping:SnapSettings,crop_mode:bool,canvas_locked:bool)->(bool,egui::Response) {
 use auroracast_core::layout::{bounds,set_crop};
 selection.retain(|&i|scene.layers.get(i).is_some_and(|l|l.visible));
 let owner=ui.id().with("canvas-edit");let response=ui.interact(canvas.expand(10.).intersect(ui.clip_rect()),owner,egui::Sense::click_and_drag());let mut changed=false;let mut guides=[None,None];
 if drag.as_ref().is_some_and(|d|d.owner==owner&&d.scene!=scene.id){*drag=None;}
 let points=|r:Rect|[r.left_top(),r.right_top(),r.right_bottom(),r.left_bottom(),r.center_top(),r.right_center(),r.center_bottom(),r.left_center()];
 let unlocked=!canvas_locked&&selection.iter().all(|&i|!scene.layers[i].locked);
 let selected_rect=bounds(scene,selection).map(|r|screen_rect(r,canvas));
 let handle=|point:Pos2|->Option<usize>{if !unlocked{return None}points(selected_rect?).iter().position(|p|p.distance(point)<=9.)};
 if !canvas_locked&&(response.clicked()||response.drag_started())&&let Some(point)=response.interact_pointer_pos(){
  let origin=ui.input(|i|i.pointer.press_origin()).unwrap_or(point);let corner=handle(origin);let shift=ui.input(|i|i.modifiers.shift);
  let hit=scene.layers.iter().enumerate().rev().find(|(_,l)|l.visible&&screen_rect(l.rect,canvas).contains(origin)).map(|(i,_)|i);
  let prior=selection.clone();
  if corner.is_none(){if let Some(index)=hit {
   let group=scene.layers[index].group;let members:Vec<_>=scene.layers.iter().enumerate().filter(|(i,l)|*i==index||(group!=0&&l.group==group&&l.visible)).map(|(i,_)|i).collect();
   if shift {for i in members {if selection.contains(&i){selection.retain(|n|*n!=i);}else{selection.push(i);}}}
   else if !selection.contains(&index){*selection=members;}
   *selected=selection.last().copied();
  }else if !shift{selection.clear();*selected=None;}}
  if response.drag_started(){
   let marquee=corner.is_none()&&hit.is_none();
   if marquee||selection.iter().all(|&i|!scene.layers[i].locked){*drag=Some(Drag{owner,scene:scene.id,rect:bounds(scene,selection).unwrap_or([0.,0.,1.,1.]),pointer:origin,corner,originals:selection.iter().map(|&i|(i,scene.layers[i].clone())).collect(),marquee,prior:if shift{prior}else{Vec::new()}});}
  }
 }
 if !canvas_locked&&(response.dragged()||response.drag_stopped())&&let(Some(active),Some(point))=(drag.as_ref(),response.interact_pointer_pos())&&active.owner==owner&&active.scene==scene.id {
  if active.marquee {
   let rect=Rect::from_two_pos(active.pointer,point);*selection=active.prior.clone();for(i,l)in scene.layers.iter().enumerate(){if l.visible&&rect.intersects(screen_rect(l.rect,canvas))&&!selection.contains(&i){selection.push(i);}}let groups:Vec<_>=selection.iter().map(|&i|scene.layers[i].group).filter(|g|*g!=0).collect();for(i,l)in scene.layers.iter().enumerate(){if l.visible&&groups.contains(&l.group)&&!selection.contains(&i){selection.push(i);}}*selected=selection.last().copied();ui.painter().rect_filled(rect,0.,Color32::from_rgba_unmultiplied(173,145,255,25));ui.painter().rect_stroke(rect,0.,(1.,Color32::from_rgb(173,145,255)),egui::StrokeKind::Inside);
  }else if active.originals.iter().all(|(i,_)|scene.layers.get(*i).is_some_and(|l|!l.locked)){
   let d=point-active.pointer;let delta=Vec2::new(d.x/canvas.width(),d.y/canvas.height());
   if crop_mode&&active.originals.len()==1&&let Some(edge)=active.corner {
    let(index,original)=&active.originals[0];let mut layer=original.clone();let mut crop=original.crop;let full=[original.rect[2]/(1.-crop[0]-crop[2]),original.rect[3]/(1.-crop[1]-crop[3])];
    if [0,3,7].contains(&edge){crop[0]=(crop[0]+delta.x/full[0]).clamp(0.,0.99-crop[2]);}if [1,2,5].contains(&edge){crop[2]=(crop[2]-delta.x/full[0]).clamp(0.,0.99-crop[0]);}if [0,1,4].contains(&edge){crop[1]=(crop[1]+delta.y/full[1]).clamp(0.,0.99-crop[3]);}if [2,3,6].contains(&edge){crop[3]=(crop[3]-delta.y/full[1]).clamp(0.,0.99-crop[1]);}set_crop(&mut layer,crop);changed|=scene.layers[*index].crop!=layer.crop;scene.layers[*index]=layer;
   }else{
    // Alt stretches one side only; otherwise every handle keeps the layer's proportions.
    let aspect=aspect&&!ui.input(|i|i.modifiers.alt);
    let raw=transform(active.rect,delta,active.corner,aspect);let mut settings=snapping;settings.enabled&=!ui.input(|i|i.modifiers.ctrl);
    let mut obstacles=scene.clone();obstacles.layers=scene.layers.iter().enumerate().filter(|(i,_)|!active.originals.iter().any(|(n,_)|i==n)).map(|(_,l)|l.clone()).collect();
    let result=crate::snapping::Snapper::new(&obstacles,usize::MAX,[canvas.width(),canvas.height()],settings).apply(raw,active.rect,active.corner,aspect&&active.corner.is_none_or(|c|c<4));guides=result.guides;
    let mut result=result;if aspect&&let Some(edge)=active.corner.filter(|c|*c>=4){result.rect=keep_edge_aspect(result.rect,active.rect,edge);}
    for(index,original)in &active.originals {let next=[result.rect[0]+(original.rect[0]-active.rect[0])*result.rect[2]/active.rect[2],result.rect[1]+(original.rect[1]-active.rect[1])*result.rect[3]/active.rect[3],original.rect[2]*result.rect[2]/active.rect[2],original.rect[3]*result.rect[3]/active.rect[3]];changed|=scene.layers[*index].rect!=next;scene.layers[*index].rect=next;}
   }
  }
 }
 if drag.as_ref().is_some_and(|d|d.owner==owner)&&(response.drag_stopped()||!ui.input(|i|i.pointer.primary_down())){*drag=None;}
 let painter=ui.painter().with_clip_rect(canvas.expand(8.));let color=if !unlocked{Color32::GRAY}else if crop_mode{Color32::from_rgb(255,191,90)}else{Color32::from_rgb(173,145,255)};
 for &i in selection.iter(){painter.rect_stroke(screen_rect(scene.layers[i].rect,canvas),0.,(1.,color),egui::StrokeKind::Inside);}
 if let Some(r)=bounds(scene,selection).map(|r|screen_rect(r,canvas)){painter.rect_stroke(r,0.,(1.5,color),egui::StrokeKind::Inside);if unlocked{for point in points(r){painter.rect_filled(Rect::from_center_size(point,Vec2::splat(8.)),2.,color);}}}
 if response.dragged(){let stroke=egui::Stroke::new(1.0_f32,Color32::from_rgb(67,221,222));if let Some(x)=guides[0]{let x=canvas.left()+x*canvas.width();painter.line_segment([egui::pos2(x,canvas.top()),egui::pos2(x,canvas.bottom())],stroke);}if let Some(y)=guides[1]{let y=canvas.top()+y*canvas.height();painter.line_segment([egui::pos2(canvas.left(),y),egui::pos2(canvas.right(),y)],stroke);}}
 if response.hovered()&&let Some(point)=ui.input(|i|i.pointer.hover_pos())&&let Some(h)=handle(point){ui.ctx().set_cursor_icon(match h{0|2=>egui::CursorIcon::ResizeNwSe,1|3=>egui::CursorIcon::ResizeNeSw,4|6=>egui::CursorIcon::ResizeVertical,_=>egui::CursorIcon::ResizeHorizontal});}
 (changed,response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_previews_keep_vertical_drag_alive_across_multiple_frames() {
        let ctx = egui::Context::default();
        let mut scenes = auroracast_core::Project::default().scenes;
        scenes.truncate(2);
        for scene in &mut scenes {
            scene.layers.truncate(1);
            scene.layers[0].rect = [0.25, 0.25, 0.5, 0.5];
            scene.layers[0].visible = true;
            scene.layers[0].locked = false;
        }
        let main_before = scenes[0].layers[0].rect;
        let mut selections = [None, Some(0)];
        let mut drag = None;
        let mut frame = |events: Vec<egui::Event>, reverse: bool| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800., 500.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for i in if reverse { [1, 0] } else { [0, 1] } {
                        ui.push_id(i, |ui| {
                            let rect = if i == 0 {
                                Rect::from_min_size(egui::pos2(30., 50.), Vec2::new(400., 225.))
                            } else {
                                Rect::from_min_size(egui::pos2(500., 50.), Vec2::new(200., 400.))
                            };
                            edit(
                                ui,
                                rect,
                                &mut scenes[i],
                                &mut selections[i],
                                &mut drag,
                                true,
                                SnapSettings {
                                    enabled: false,
                                    ..Default::default()
                                },
                            );
                        });
                    }
                });
            });
            (
                scenes[0].layers[0].rect,
                scenes[1].layers[0].rect,
                drag.is_some(),
            )
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![], false);
        let corner = egui::pos2(650., 350.);
        frame(
            vec![egui::Event::PointerMoved(corner), button(corner, true)],
            false,
        );
        for (point, size, reverse) in [
            (egui::pos2(660., 370.), 0.55, false),
            (egui::pos2(670., 390.), 0.6, true),
            (egui::pos2(680., 410.), 0.65, false),
        ] {
            let (main, vertical, active) = frame(vec![egui::Event::PointerMoved(point)], reverse);
            assert_eq!(main, main_before);
            assert!(active, "Other preview canceled the vertical drag");
            assert!((vertical[2] - size).abs() < 0.001, "{vertical:?}");
            assert!((vertical[3] - size).abs() < 0.001);
            assert_eq!([vertical[0], vertical[1]], [0.25, 0.25]);
        }
        let (_, vertical, active) = frame(vec![button(egui::pos2(680., 410.), false)], false);
        assert!(!active);
        assert!((vertical[2] - 0.65).abs() < 0.001);
    }

    #[test]
    fn edge_handle_can_be_grabbed_outside_canvas_image() {
        let ctx = egui::Context::default();
        let mut scene = auroracast_core::Project::default().scenes.remove(0);
        scene.layers.truncate(1);
        scene.layers[0].rect = [0., 0., 1., 1.];
        scene.layers[0].visible = true;
        scene.layers[0].locked = false;
        let mut selected = Some(0);
        let mut drag = None;
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(500., 600.))),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        edit(
                            ui,
                            Rect::from_min_size(egui::pos2(100., 50.), Vec2::new(200., 400.)),
                            &mut scene,
                            &mut selected,
                            &mut drag,
                            true,
                            SnapSettings {
                                enabled: false,
                                ..Default::default()
                            },
                        );
                    });
                },
            );
            scene.layers[0].rect
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![]);
        let start = egui::pos2(303., 453.);
        frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
        frame(vec![egui::Event::PointerMoved(egui::pos2(283., 413.))]);
        let rect = frame(vec![button(egui::pos2(283., 413.), false)]);
        assert!((rect[2] - 0.9).abs() < 0.001, "{rect:?}");
        assert!((rect[3] - 0.9).abs() < 0.001);
    }

    #[test]
    fn snapping_ctrl_bypass_releases_without_drift_and_guides_disappear() {
        let ctx = egui::Context::default();
        let mut scene = auroracast_core::Project::default().scenes.remove(0);
        scene.layers.truncate(1);
        scene.layers[0].rect = [0.2, 0.2, 0.2, 0.2];
        scene.layers[0].locked = false;
        scene.layers[0].visible = true;
        let mut selected = Some(0);
        let mut drag = None;
        let guide_color = Color32::from_rgb(67, 221, 222);
        let mut frame = |events: Vec<egui::Event>, ctrl: bool| {
            let modifiers = egui::Modifiers {
                ctrl,
                ..Default::default()
            };
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600., 400.))),
                events,
                modifiers,
                ..Default::default()
            };
            let output = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    edit(
                        ui,
                        Rect::from_min_size(egui::pos2(50., 50.), Vec2::new(400., 200.)),
                        &mut scene,
                        &mut selected,
                        &mut drag,
                        true,
                        SnapSettings::default(),
                    );
                });
            });
            let has_guide=output.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::LineSegment {stroke,..} if stroke.color==guide_color));
            (scene.layers[0].rect, has_guide)
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![], false);
        let start = egui::pos2(170., 110.); // Middle of source: x=.3, y=.3
        frame(
            vec![egui::Event::PointerMoved(start), button(start, true)],
            false,
        );
        let near = egui::pos2(94., 110.); // Raw left edge 4 px from canvas
        let (r, guide) = frame(vec![egui::Event::PointerMoved(near)], false);
        assert!(r[0].abs() < 0.0001);
        assert!(guide);
        let (r, guide) = frame(vec![], true);
        assert!((r[0] - 0.01).abs() < 0.0001);
        assert!(!guide);
        let (r, guide) = frame(vec![], false);
        assert!(r[0].abs() < 0.0001);
        assert!(guide);
        let (r, guide) = frame(
            vec![egui::Event::PointerMoved(egui::pos2(110., 110.))],
            false,
        );
        assert!((r[0] - 0.05).abs() < 0.0001);
        assert!(!guide);
        let (_, guide) = frame(vec![button(egui::pos2(110., 110.), false)], false);
        assert!(!guide);
    }

    #[test]
    fn mouse_drag_resizes_and_respects_lock() {
        let ctx = egui::Context::default();
        let mut scene = auroracast_core::Project::default().scenes.remove(0);
        scene.layers.truncate(1);
        scene.layers[0].rect = [0.25, 0.25, 0.5, 0.5];
        scene.layers[0].locked = false;
        scene.layers[0].visible = true;
        let mut selected = Some(0);
        let mut drag = None;
        let mut frame = |events: Vec<egui::Event>, scene: &mut Scene| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600., 400.))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    edit(
                        ui,
                        Rect::from_min_size(egui::pos2(50., 50.), Vec2::new(400., 200.)),
                        scene,
                        &mut selected,
                        &mut drag,
                        true,
                        SnapSettings::default(),
                    );
                });
            });
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![], &mut scene);
        let start = egui::pos2(350., 200.);
        let end = egui::pos2(390., 220.);
        frame(
            vec![egui::Event::PointerMoved(start), button(start, true)],
            &mut scene,
        );
        frame(vec![egui::Event::PointerMoved(end)], &mut scene);
        frame(vec![button(end, false)], &mut scene);
        assert!(
            (scene.layers[0].rect[2] - 0.6).abs() < 0.001,
            "{:?}",
            scene.layers[0].rect
        );
        assert!((scene.layers[0].rect[3] - 0.6).abs() < 0.001);
        let before = scene.layers[0].rect;
        scene.layers[0].locked = true;
        frame(
            vec![egui::Event::PointerMoved(end), button(end, true)],
            &mut scene,
        );
        frame(
            vec![egui::Event::PointerMoved(egui::pos2(430., 250.))],
            &mut scene,
        );
        frame(vec![button(egui::pos2(430., 250.), false)], &mut scene);
        assert_eq!(scene.layers[0].rect, before);
    }

    #[test]
    fn corners_preserve_opposite_anchor_and_aspect() {
        for corner in 0..4 {
            let r = [0.2, 0.1, 0.4, 0.3];
            let out = transform(r, Vec2::new(0.1, 0.07), Some(corner), true);
            assert!((out[2] / out[3] - r[2] / r[3]).abs() < 0.0001);
            let left = corner == 0 || corner == 3;
            let top = corner < 2;
            assert!(
                (out[0] + if left { out[2] } else { 0. } - (r[0] + if left { r[2] } else { 0. }))
                    .abs()
                    < 0.0001
            );
            assert!(
                (out[1] + if top { out[3] } else { 0. } - (r[1] + if top { r[3] } else { 0. }))
                    .abs()
                    < 0.0001
            );
        }
    }
    #[test]
    fn crossing_anchor_cannot_flip_or_destroy_item() {
        let out = transform([0., 0., 0.5, 0.5], Vec2::splat(-10.), Some(2), false);
        assert_eq!(out, [0., 0., 0.005, 0.005]);
        let out = transform([0., 0., 0.5, 0.5], Vec2::splat(10.), None, true);
        assert_eq!(out, [0.99, 0.99, 0.5, 0.5]);
    }
}

#[cfg(test)] mod selection_tests {
 use super::*;
 #[test]fn middle_edges_scale_proportionally_around_center(){let r=[0.2,0.3,0.4,0.5];for edge in 4..8{let next=transform(r,Vec2::new(0.1,0.1),Some(edge),true);assert!((next[2]/next[3]-0.8).abs()<0.0001,"edge {edge} distorted the layer");let c=if edge==4||edge==6{next[0]+next[2]/2.}else{next[1]+next[3]/2.};assert!((c-if edge==4||edge==6{0.4}else{0.55}).abs()<0.0001);if edge==4{assert!((next[1]+next[3]-0.8).abs()<0.0001);}if edge==7{assert!((next[0]+next[2]-0.6).abs()<0.0001);}}}
 #[test]fn middle_edges_preserve_other_axis_and_opposite_edge(){let r=[0.2,0.3,0.4,0.5];for edge in 4..8{let next=transform(r,Vec2::new(0.1,0.1),Some(edge),false);if edge==4||edge==6{assert_eq!([next[0],next[2]],[r[0],r[2]]);}else{assert_eq!([next[1],next[3]],[r[1],r[3]]);}if edge==4{assert!((next[1]+next[3]-0.8).abs()<0.0001);}if edge==7{assert!((next[0]+next[2]-0.6).abs()<0.0001);}}}
 #[test]fn grouped_drag_and_canvas_lock(){
  let ctx=egui::Context::default();let mut scene=auroracast_core::Project::default().scenes.remove(0);scene.layers.truncate(1);scene.layers[0].rect=[0.1,0.1,0.2,0.2];scene.layers[0].group=1;scene.layers[0].visible=true;scene.layers[0].locked=false;let mut second=scene.layers[0].clone();second.rect[0]=0.5;scene.layers.push(second);
  let mut selected=None;let mut selection=vec![];let mut drag=None;
  let mut frame=|events,locked|{let _=ctx.run(egui::RawInput{screen_rect:Some(Rect::from_min_size(Pos2::ZERO,Vec2::new(600.,400.))),events,..Default::default()},|ctx|{egui::CentralPanel::default().show(ctx,|ui|{edit_multi(ui,Rect::from_min_size(egui::pos2(50.,50.),Vec2::new(400.,200.)),&mut scene,&mut selected,&mut selection,&mut drag,true,SnapSettings{enabled:false,..Default::default()},false,locked);});});(scene.layers.iter().map(|l|l.rect).collect::<Vec<_>>(),selection.clone())};
  let button=|pos,pressed|egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:egui::Modifiers::NONE};let start=egui::pos2(130.,90.);let end=egui::pos2(170.,110.);
  frame(vec![],false);frame(vec![egui::Event::PointerMoved(start),button(start,true)],false);frame(vec![egui::Event::PointerMoved(end)],false);let(r,selected)=frame(vec![button(end,false)],false);assert_eq!(selected,vec![0,1]);assert!((r[0][0]-0.2).abs()<0.001);assert!((r[1][0]-0.6).abs()<0.001);
  frame(vec![button(end,true)],true);frame(vec![egui::Event::PointerMoved(start)],true);let(after,_)=frame(vec![button(start,false)],true);assert_eq!(after,r);
 }
}
