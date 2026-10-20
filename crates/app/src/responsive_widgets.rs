#[cfg(test)] mod responsive_tests{
 use super::*;
 #[test]fn widgets_reflow_to_requested_bounds(){
  for(width,height)in[(960,160),(360,720),(600,400)]{for kind in[Kind::Chat,Kind::NowPlaying,Kind::Sports,Kind::SubGoal]{let mut s=Settings{kind,label:kind.label().into(),layout_width:width,layout_height:height,..Default::default()};s.music.automatic=false;let frame=preview(&s,true).unwrap();assert_eq!((frame.width,frame.height),(width,height));assert_eq!(frame.rgba.len(),(width*height*4)as usize);assert!(frame.rgba.chunks_exact(4).any(|p|p[3]>0));if let Ok(dir)=std::env::var("ROSORAH_RESPONSIVE_QA"){std::fs::create_dir_all(&dir).unwrap();image::save_buffer(std::path::Path::new(&dir).join(format!("{kind:?}-{width}-{height}.png")),&frame.rgba,width,height,image::ColorType::Rgba8).unwrap();}}}
 }
}

