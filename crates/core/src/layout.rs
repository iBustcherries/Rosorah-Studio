//! Layer-space operations used by preview editing and project tools.
use crate::{Layer,Scene};
pub fn bounds(scene:&Scene,indices:&[usize])->Option<[f32;4]>{
 let mut b=[f32::INFINITY,f32::INFINITY,f32::NEG_INFINITY,f32::NEG_INFINITY];let mut any=false;
 for &i in indices{if let Some(l)=scene.layers.get(i){any=true;b[0]=b[0].min(l.rect[0]);b[1]=b[1].min(l.rect[1]);b[2]=b[2].max(l.rect[0]+l.rect[2]);b[3]=b[3].max(l.rect[1]+l.rect[3]);}}
 any.then(||[b[0],b[1],(b[2]-b[0]).max(0.0001),(b[3]-b[1]).max(0.0001)])
}
pub fn set_crop(layer:&mut Layer,mut crop:[f32;4]) {
 for v in &mut crop{*v=if v.is_finite(){v.clamp(0.,0.99)}else{0.};}
 for axis in 0..2{if crop[axis]+crop[axis+2]>0.99{crop[axis+2]=0.99-crop[axis];}}
 for axis in 0..2 {let full=layer.rect[axis+2]/(1.-layer.crop[axis]-layer.crop[axis+2]).max(0.01);layer.rect[axis]+=(crop[axis]-layer.crop[axis])*full;layer.rect[axis+2]=full*(1.-crop[axis]-crop[axis+2]);}
 layer.crop=crop;
}
pub fn distribute(scene:&mut Scene,indices:&[usize],axis:usize){
 let mut indices:Vec<_>=indices.iter().copied().filter(|i|scene.layers.get(*i).is_some_and(|l|!l.locked)).collect();if indices.len()<3{return}
 indices.sort_by(|&a,&b|scene.layers[a].rect[axis].total_cmp(&scene.layers[b].rect[axis]));
 let first=scene.layers[indices[0]].rect[axis];let last=&scene.layers[*indices.last().unwrap()];let end=last.rect[axis]+last.rect[axis+2];let widths:f32=indices.iter().map(|&i|scene.layers[i].rect[axis+2]).sum();let gap=(end-first-widths)/(indices.len()-1)as f32;let mut x=first;
 for i in indices{scene.layers[i].rect[axis]=x;x+=scene.layers[i].rect[axis+2]+gap;}
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn crop_preserves_scale_and_reset_restores_rect(){let mut p=crate::Project::default();let l=&mut p.scenes[0].layers[0];l.rect=[0.1,0.2,0.6,0.4];let old=l.rect;set_crop(l,[0.1,0.2,0.3,0.1]);assert!((l.rect[0]-0.16).abs()<0.0001);assert!((l.rect[2]-0.36).abs()<0.0001);set_crop(l,[0.;4]);for(a,b)in l.rect.iter().zip(old){assert!((*a-b).abs()<0.0001)}}
 #[test]fn distribution_uses_equal_gaps(){let mut s=crate::Project::default().scenes.remove(0);let l=s.layers[0].clone();s.layers=vec![l.clone(),l.clone(),l];s.layers[0].rect=[0.,0.,0.1,0.1];s.layers[1].rect=[0.2,0.,0.2,0.1];s.layers[2].rect=[0.9,0.,0.1,0.1];distribute(&mut s,&[0,1,2],0);assert!((s.layers[1].rect[0]-0.4).abs()<0.0001);}
}
