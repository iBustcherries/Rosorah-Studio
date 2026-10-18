//! Match camera proportions when a new input format arrives, without fighting edits.
use auroracast_core::{Id, Project, SourceKind};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct AutoAspect(BTreeMap<(Id, usize, Id), [u32; 4]>);
impl AutoAspect {
    pub fn update(&mut self, project: &mut Project, frames: &BTreeMap<Id, [u32; 2]>) -> bool {
        let mut next = BTreeMap::new();
        let mut changed = false;
        for scene in &mut project.scenes {
            let Some(canvas) = project.canvases.iter().find(|c| c.scene == scene.id) else {
                continue;
            };
            for (index, layer) in scene.layers.iter_mut().enumerate() {
                if layer.locked || !project.sources.iter().any(|s| s.id == layer.source && matches!(&s.kind, SourceKind::VideoCamera(c) if c.auto_aspect && c.enabled)) { continue; }
                let Some(&[width, height]) = frames.get(&layer.source) else {
                    continue;
                };
                if [width, height, canvas.width, canvas.height].contains(&0) {
                    continue;
                }
                let key = (scene.id, index, layer.source);
                let dimensions = [width, height, canvas.width, canvas.height];
                next.insert(key, dimensions);
                if self.0.get(&key) == Some(&dimensions) {
                    continue;
                }
                let h = layer.rect[2] * canvas.width as f32 / canvas.height as f32 * height as f32
                    / width as f32;
                if h.is_finite() && h > 0. && (layer.rect[3] - h).abs() > 0.000001 {
                    layer.rect[3] = h;
                    changed = true;
                }
            }
        }
        self.0 = next;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auroracast_core::{CameraSettings, Layer, Source};
    #[test]
    fn follows_actual_camera_dimensions_on_landscape_and_portrait() {
        let mut p = Project::default();
        let id = p.id();
        p.sources.push(Source {
            id,
            name: "DroidCam".into(),
            kind: SourceKind::VideoCamera(CameraSettings {
                enabled: true,
                ..Default::default()
            }),
        });
        for i in 0..2 {
            let scene = p.canvases[i].scene;
            p.scenes.iter_mut().find(|s| s.id == scene).unwrap().layers = vec![Layer { crop: [0.;4], group: 0,
                source: id,
                rect: [0.2, 0.1, 0.3, 0.3],
                visible: true,
                locked: false,
                opacity: 1.,
            }];
        }
        let mut fit = AutoAspect::default();
        for dimensions in [[640, 480], [1920, 1080], [1080, 1920]] {
            assert!(fit.update(&mut p, &BTreeMap::from([(id, dimensions)])));
            for canvas in p.canvases.iter().take(2) {
                let layer = &p
                    .scenes
                    .iter()
                    .find(|s| s.id == canvas.scene)
                    .unwrap()
                    .layers[0];
                let ratio =
                    layer.rect[2] * canvas.width as f32 / (layer.rect[3] * canvas.height as f32);
                assert!((ratio - dimensions[0] as f32 / dimensions[1] as f32).abs() < 0.0001);
                assert_eq!(
                    [layer.rect[0], layer.rect[1], layer.rect[2]],
                    [0.2, 0.1, 0.3]
                );
            }
            assert!(!fit.update(&mut p, &BTreeMap::from([(id, dimensions)])));
        }
        let scene = p.canvases[0].scene;
        p.scenes.iter_mut().find(|s| s.id == scene).unwrap().layers[0].rect[3] = 0.9;
        assert!(
            !fit.update(&mut p, &BTreeMap::from([(id, [1080, 1920])])),
            "Stable input must not fight manual resizing"
        );
        p.scenes.iter_mut().find(|s| s.id == scene).unwrap().layers[0].locked = true;
        fit.update(&mut p, &BTreeMap::from([(id, [640, 480])]));
        assert_eq!(
            p.scenes.iter().find(|s| s.id == scene).unwrap().layers[0].rect[3],
            0.9
        );
        let SourceKind::VideoCamera(settings) =
            &mut p.sources.iter_mut().find(|s| s.id == id).unwrap().kind
        else {
            panic!()
        };
        settings.auto_aspect = false;
        assert!(!fit.update(&mut p, &BTreeMap::from([(id, [1920, 1080])])));
    }
}
