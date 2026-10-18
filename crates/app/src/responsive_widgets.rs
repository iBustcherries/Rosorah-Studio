fn sized_canvas(s:&Settings)->Paint{let width=s.layout_width.clamp(96,960);let height=s.layout_height.clamp(96,960);Paint{pixels:vec![0;(width*height*4)as usize],width,height}}
fn text_width(text:&str,size:f32,face:FontFace)->f32{let f=font(face).as_scaled(PxScale::from(size));text.chars().map(|c|f.h_advance(f.glyph_id(c))).sum()}
fn wrap_words(text:&str,width:f32,size:f32,face:FontFace)->Vec<String>{let mut rows=vec![String::new()];for word in text.split_whitespace(){let last=rows.last_mut().unwrap();if !last.is_empty()&&text_width(&format!("{last} {word}"),size,face)>width{rows.push(word.into());}else{if !last.is_empty(){last.push(' ');}last.push_str(word);}}rows}
fn polished_chat(s:&Settings,text:&str)->Frame {
 if matches!(s.theme,Theme::Windows98|Theme::WindowsXP)&&s.layout_width==960&&s.layout_height==400{return legacy_chat(s,text)}
 let mut p=sized_canvas(s);let w=p.width as f32;let h=p.height as f32;let pad=16.;let size=(s.font_size as f32).min(w/12.).clamp(14.,48.);let retro=matches!(s.theme,Theme::Windows98|Theme::WindowsXP);let header=if retro{42.}else if s.label.is_empty(){0.}else{36.};let bubbles=s.theme==Theme::Bubbles||s.layout==auroracast_core::widgets::Layout::Bubbles;
 p.soft_rect(0.,0.,w,h,s.radius as f32,s.background);if retro{for y in 0..36{p.rect(3,3+y,(w-6.)as i32,1,0,tint(s.accent,(12-y/2)as i16));}p.line(if s.label.is_empty(){"Live chat"}else{&s.label},12.,27.,w-70.,18.,[255;4],s.font,true);p.soft_rect(w-32.,10.,22.,22.,2.,tint(s.background,20));p.line("×",w-28.,27.,18.,18.,s.foreground,s.font,true);}else if header>0.{p.line(&s.label,pad,25.,w-pad*2.,19.,s.foreground,s.font,true);}
 let max_height=(h-header-pad*2.).max(35.);let gap=s.spacing as f32;let mut rows=Vec::new();let mut total=0.;
 for row in text.lines().skip(1).collect::<Vec<_>>().into_iter().rev(){if row.is_empty(){continue}let(platform,rest)=row.split_once(" · ").unwrap_or(("",row));let(author,body)=rest.split_once(": ").unwrap_or(("",rest));let lines=wrap_words(body,w-pad*4.,size,s.font);let lines:Vec<_>=lines.into_iter().take(5).collect();let height=(37.+lines.len()as f32*size*1.25).min(max_height);if total+height>max_height&& !rows.is_empty(){break}total+=height+gap;rows.push((platform,author,body,lines,height));}
 if !s.reverse{rows.reverse();}let mut y=if s.reverse{header+pad}else{(h-total).max(header+pad)};
 for(i,(platform,author,body,lines,height))in rows.into_iter().enumerate(){let color=if s.theme==Theme::Unified{if platform=="Twitch"{[179,137,255,255]}else{[255,123,134,255]}}else{s.accent};let width=if bubbles{(text_width(body,size,s.font)+36.).clamp((w*0.45).min(200.),w-pad*2.)}else{w-pad*2.};let x=if bubbles&&i%2==1{w-pad-width}else{pad};let fill=if bubbles{if i%2==1{s.accent}else{[45,48,56,240]}}else{tint(s.background,12)};
 if s.layout!=auroracast_core::widgets::Layout::Lines{p.soft_rect(x,y,width,height,s.radius as f32,fill);if !bubbles{p.soft_rect(x,y+10.,3.,height-20.,1.5,color);}}
 p.line(author,x+14.,y+22.,(width-110.).max(40.),16.,color,s.font,true);p.line(platform,x+width-87.,y+22.,75.,12.,subdued(s.foreground),s.font,false);
 for(j,line)in lines.iter().enumerate(){let py=y+28.+size+j as f32*size*1.25;if py>y+height-3.{break}p.line(line,x+14.,py,width-28.,size,s.foreground,s.font,s.bold);}y+=height+gap;
 }p.finish()
}
fn polished_music(s:&Settings,track:Option<&crate::music::Track>)->Frame{
 if s.layout_width==960&&s.layout_height==400{return legacy_music(s,track)}
 let mut p=sized_canvas(s);let Some(t)=track else{return p.finish()};let w=p.width as f32;let h=p.height as f32;let portrait=w/h<1.35;let pad=(w.min(h)*0.07).clamp(10.,28.);let art=if portrait{(w-pad*2.).min(h*0.57)}else{(h-pad*2.).min(w*0.32)};let ax=if portrait{(w-art)/2.}else{pad};let ay=pad;let accent=if s.music.artwork_colors{t.accent}else{s.accent};p.soft_rect(0.,0.,w,h,s.radius as f32,s.background);
 for yy in 0..art as u32{for xx in 0..art as u32{let c=if let Some(f)=&t.artwork{let ix=xx*f.width/art as u32;let iy=yy*f.height/art as u32;let o=((iy*f.width+ix)*4)as usize;f.rgba[o..o+4].try_into().unwrap()}else{let k=0.45+0.3*(xx+yy)as f32/(art*2.);[(accent[0]as f32*k)as u8,(accent[1]as f32*k)as u8,(accent[2]as f32*k)as u8,255]};p.blend(ax as i32+xx as i32,ay as i32+yy as i32,c,1.);}}
 let x=if portrait{pad}else{ax+art+pad};let top=if portrait{ay+art+pad+15.}else{pad+12.};let width=w-x-pad;let compact=h<180.;let title_size=(s.font_size as f32).min(width/10.).clamp(16.,48.);
 if !compact{p.line(if t.playing{"NOW PLAYING"}else{"PAUSED"},x,top,width,14.,accent,s.font,true);}
 let ty=top+if compact{6.}else{title_size+12.};p.line(&t.title,x,ty,width,title_size,s.foreground,s.font,true);let artist_size=if s.music.artist_size>0{s.music.artist_size as f32}else{(title_size*0.65).max(14.)};let artist_y=ty+(title_size*0.2+artist_size*0.9).max(title_size*0.8);p.line_font(&t.artist,x,artist_y,width,artist_size,subdued(s.foreground),&font_or(&s.music.artist_font,s.font),false);
 if s.music.show_album&&!t.album.is_empty(){let album_size=if s.music.album_size>0{s.music.album_size as f32}else{(artist_size*0.8).max(12.)};p.line_font(&t.album,x,artist_y+artist_size*0.15+album_size*1.15,width,album_size,subdued(s.foreground),&font_or(&s.music.album_font,s.font),false);}
 if s.music.progress&&t.length>0.&&h>120.{let py=h-pad-24.;p.soft_rect(x,py,width,4.,2.,subdued(tint(s.background,65)));p.soft_rect(x,py,width*t.progress(),4.,2.,accent);let seconds=(t.progress()as f64*t.length)as u64;p.line(&format!("{}:{:02} / {}:{:02}",seconds/60,seconds%60,t.length as u64/60,t.length as u64%60),x,py+20.,width,13.,subdued(s.foreground),s.font,false);}p.finish()
}
fn responsive_sports(s:&Settings,text:&str,elapsed:f32)->Frame{
 let mut p=sized_canvas(s);let w=p.width as f32;let h=p.height as f32;let(header,body)=text.split_once('\n').unwrap_or((text,""));let pad=14.;p.soft_rect(0.,0.,w,h,s.radius as f32,s.background);
 p.soft_rect(0.,0.,5.,h,2.,s.accent);p.line(header,pad,24.,w-2.*pad,15.,subdued(s.foreground),s.font,true);
 if s.sports.layout==ScoreLayout::Ticker{let size=(s.font_size as f32).min((h-40.)*0.55).max(16.);let length=text_width(body,size,s.font);let cycle=length+w*0.2;let offset=(elapsed*s.sports.speed as f32)%cycle.max(1.);let first=if s.sports.rightward{offset-cycle}else{-offset};for x in[first,first+cycle]{p.text(body,x+pad,(h+35.)*0.65,length+1.,size,s.foreground,s.font,s.bold,h-pad);}return p.finish()}
 if !body.contains(" · "){let size=(s.font_size as f32).clamp(16.,28.);let mut y=(h*0.5).max(60.);for line in wrap_words(body,w-pad*2.,size,s.font){p.line(&line,pad,y,w-pad*2.,size,subdued(s.foreground),s.font,false);y+=size*1.3;}return p.finish()}
 let rows:Vec<_>=body.lines().take(5).collect();let row_h=(h-42.)/rows.len().max(1)as f32;
 for(i,row)in rows.iter().enumerate(){let y=38.+i as f32*row_h;let parts:Vec<_>=row.split(" · ").collect();let league=if parts.len()>1{parts[0]}else{"SCORES"};let main=if parts.len()>1{parts[1]}else{row};let status=parts.iter().skip(2).copied().collect::<Vec<_>>().join(" · ");p.soft_rect(pad,y,w-2.*pad,row_h-5.,8.,tint(s.background,9));p.line(league,pad+14.,y+20.,w*0.4,14.,s.accent,s.font,true);p.line(&status,w*0.55,y+20.,w*0.45-2.*pad,14.,subdued(s.foreground),s.font,false);let size=(s.font_size as f32).max(row_h*0.29).min(row_h*0.4).clamp(16.,72.);p.line(main,pad+14.,y+(row_h+size)*0.60,w-4.*pad,size,s.foreground,s.font,true);}
 p.finish()
}
fn polished_goal(s:&Settings,value:u64)->Frame{
 if s.layout_width==960&&s.layout_height==400{return legacy_goal(s,value)}
 let mut p=sized_canvas(s);let w=p.width as f32;let h=p.height as f32;let pad=18.;let compact=h<180.;let accent=s.accent;let dim=[accent[0],accent[1],accent[2],50];let t=(value as f64/s.target.max(1)as f64).clamp(0.,1.)as f32;
 p.soft_rect(0.,0.,w,h,s.radius as f32,s.background);p.line(&s.label,pad,if compact{20.}else{36.},w-pad*2.,if compact{15.}else{24.},subdued(s.foreground),s.font,true);
 let amount=|v:u64|if s.kind==Kind::DonationGoal{format!("{}.{:02}",v/100,v%100)}else{v.to_string()};let size=if compact{28.}else{(w/9.).clamp(32.,64.)};p.line(&format!("{} / {}{}",amount(value),amount(s.target),if s.kind==Kind::DonationGoal{format!(" {}",s.currency)}else{String::new()}),pad,if compact{55.}else{50.+size},w-pad*2.,size,s.foreground,s.font,true);
 let top=if compact{70.}else{size+85.};let bottom=h-pad-if compact{0.}else{30.};let area=(bottom-top).max(12.);
 match s.goal_style{
  GoalStyle::Ring=>{let radius=(area/2.-5.).min(w/2.-pad).max(5.);let cx=w/2.;let cy=top+area/2.;let thick=(radius*0.12).clamp(3.,14.);for y in (cy-radius-thick)as i32..(cy+radius+thick)as i32{for x in (cx-radius-thick)as i32..(cx+radius+thick)as i32{let dx=x as f32-cx;let dy=y as f32-cy;let coverage=(thick-(dx.hypot(dy)-radius).abs()).clamp(0.,1.);if coverage>0.{let angle=(dy.atan2(dx)+std::f32::consts::FRAC_PI_2).rem_euclid(std::f32::consts::TAU)/std::f32::consts::TAU;p.blend(x,y,if angle<t{accent}else{dim},coverage);}}}}
  GoalStyle::Vertical|GoalStyle::Thermometer=>{p.soft_rect(w*0.45,top,w*0.1,area,w*0.05,dim);p.soft_rect(w*0.45,bottom-area*t,w*0.1,area*t,w*0.05,accent);}
  GoalStyle::Segments|GoalStyle::Milestones=>{let unit=(w-pad*2.)/10.;for i in 0..10{p.soft_rect(pad+i as f32*unit,top+area*0.4,unit-4.,(area*0.2).clamp(6.,30.),4.,if (i as f32)<t*10.{accent}else{dim});}}
  GoalStyle::Hearts|GoalStyle::Garden=>{let unit=(w-pad*2.)/10.;for i in 0..10{let x=pad+i as f32*unit;let c=if(i as f32)<t*10.{accent}else{dim};let d=unit.min(area).min(50.)*0.65;let y=top+area*0.4;if s.goal_style==GoalStyle::Garden{p.soft_rect(x+d*0.45,y,d*0.1,d,2.,c);p.soft_rect(x,y,d*0.5,d*0.5,d*0.25,c);p.soft_rect(x+d*0.5,y-d*0.2,d*0.5,d*0.5,d*0.25,c);}else{p.soft_rect(x,y,d*0.52,d*0.52,d*0.2,c);p.soft_rect(x+d*0.45,y,d*0.52,d*0.52,d*0.2,c);for yy in 0..(d*0.5)as i32{p.rect(x as i32+yy,y as i32+(d*0.4)as i32+yy,d as i32-yy*2,1,0,c);}}}}
  GoalStyle::Counter=>{},_=>{p.soft_rect(pad,top+area*0.4,w-pad*2.,(area*0.2).clamp(5.,22.),10.,dim);p.soft_rect(pad,top+area*0.4,(w-pad*2.)*t,(area*0.2).clamp(5.,22.),10.,accent);}
 }if !compact{p.line(&format!("{}% COMPLETE",(t*100.)as u32),pad,h-14.,w-pad*2.,16.,accent,s.font,true);}p.finish()
}

#[cfg(test)] mod responsive_tests{
 use super::*;
 #[test]fn widgets_reflow_to_requested_bounds(){
  for(width,height)in[(960,160),(360,720),(600,400)]{for kind in[Kind::Chat,Kind::NowPlaying,Kind::Sports,Kind::SubGoal]{let mut s=Settings{kind,label:kind.label().into(),layout_width:width,layout_height:height,..Default::default()};s.music.automatic=false;let frame=preview(&s,true).unwrap();assert_eq!((frame.width,frame.height),(width,height));assert_eq!(frame.rgba.len(),(width*height*4)as usize);assert!(frame.rgba.chunks_exact(4).any(|p|p[3]>0));if let Ok(dir)=std::env::var("ROSORAH_RESPONSIVE_QA"){std::fs::create_dir_all(&dir).unwrap();image::save_buffer(std::path::Path::new(&dir).join(format!("{kind:?}-{width}-{height}.png")),&frame.rgba,width,height,image::ColorType::Rgba8).unwrap();}}}
 }
}

fn responsive_info(s:&Settings,text:&str)->Frame{
 let mut p=sized_canvas(s);if text.is_empty(){return p.finish()}let w=p.width as f32;let h=p.height as f32;let pad=18.;let compact=h<180.;let label=if s.label.is_empty(){s.kind.label()}else{&s.label};p.soft_rect(0.,0.,w,h,s.radius as f32,s.background);p.soft_rect(pad,14.,3.,18.,1.5,s.accent);p.line(label,pad+13.,29.,w-pad*2.-13.,16.,subdued(s.foreground),s.font,true);
 let body=text.strip_prefix(&s.label).unwrap_or(text).trim_start_matches([':', '\n', ' ']);let body=if body.is_empty(){text}else{body};let size=if matches!(s.kind,Kind::Timer|Kind::Counter){(s.font_size as f32).clamp(28.,80.).min((h-45.)*0.7)}else{(s.font_size as f32).clamp(16.,40.).min(if compact{24.}else{40.})};let mut y=45.+size;
 for row in body.lines(){for line in wrap_words(row,w-pad*2.,size,s.font){if y>h-8.{return p.finish()}p.line(&line,pad,y,w-pad*2.,size,s.foreground,s.font,s.bold);y+=size*1.3;}y+=8.;}p.finish()
}
