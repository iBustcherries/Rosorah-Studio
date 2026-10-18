//! Preview-space snapping. Always starts from the unsnapped pointer transform,
//! so leaving the threshold or holding Ctrl releases a snap without drift.
use auroracast_core::{Scene, SnapSettings};

#[derive(Clone, Copy)]
struct Target {
    position: f32,
    center: bool,
}

pub struct Snapper {
    targets: [Vec<Target>; 2],
    size: [f32; 2],
    distance: f32,
}

pub struct Snapped {
    pub rect: [f32; 4],
    pub guides: [Option<f32>; 2],
}

impl Snapper {
    pub fn new(scene: &Scene, selected: usize, size: [f32; 2], settings: SnapSettings) -> Self {
        let mut this = Self {
            targets: [Vec::new(), Vec::new()],
            size: size.map(|n| n.max(1.)),
            distance: settings.distance.clamp(2., 24.),
        };
        if !settings.enabled {
            return this;
        }
        for axis in 0..2 {
            if settings.canvas_edges {
                this.add(axis, 0., false);
                this.add(axis, 1., false);
            }
            if settings.canvas_center {
                this.add(axis, 0.5, true);
            }
        }
        if settings.sources {
            for (index, layer) in scene.layers.iter().enumerate() {
                if index == selected || !layer.visible || layer.opacity <= 0. {
                    continue;
                }
                for axis in 0..2 {
                    this.add(axis, layer.rect[axis], false);
                    this.add(axis, layer.rect[axis] + layer.rect[axis + 2], false);
                    this.add(axis, layer.rect[axis] + layer.rect[axis + 2] / 2., true);
                }
            }
        }
        this
    }

    fn add(&mut self, axis: usize, position: f32, center: bool) {
        // Off-canvas guides must not pull an item away from the visible canvas.
        if position.is_finite() && (0.0..=1.0).contains(&position) {
            self.targets[axis].push(Target { position, center });
        }
    }

    pub fn apply(
        &self,
        raw: [f32; 4],
        original: [f32; 4],
        corner: Option<usize>,
        aspect: bool,
    ) -> Snapped {
        let mut result = Snapped {
            rect: raw,
            guides: [None, None],
        };
        let Some(corner) = corner else {
            for axis in 0..2 {
                let mut closest = self.distance + 0.0001;
                for target in &self.targets[axis] {
                    for offset in [0., 0.5, 1.] {
                        if target.center != (offset == 0.5) {
                            continue;
                        }
                        let delta = target.position - (raw[axis] + raw[axis + 2] * offset);
                        let distance = delta.abs() * self.size[axis];
                        if distance < closest {
                            closest = distance;
                            result.rect[axis] = raw[axis] + delta;
                            result.guides[axis] = Some(target.position);
                        }
                    }
                }
            }
            return result;
        };
        if corner>=4 {
            let axis=if corner==4||corner==6{1}else{0};let near=corner==4||corner==7;let anchor=original[axis]+if near{original[axis+2]}else{0.};let moving=raw[axis]+if near{0.}else{raw[axis+2]};let mut closest=self.distance+0.0001;
            for target in &self.targets[axis]{let distance=(target.position-moving).abs()*self.size[axis];let extent=if near{anchor-target.position}else{target.position-anchor};if distance<closest&&(0.005..=4.).contains(&extent){closest=distance;result.rect[axis]=if near{target.position}else{anchor};result.rect[axis+2]=extent;result.guides[axis]=Some(target.position);}}return result;
        }
        let near = [corner == 0 || corner == 3, corner < 2];
        let anchor: [f32; 2] = std::array::from_fn(|axis| {
            original[axis] + if near[axis] { original[axis + 2] } else { 0. }
        });
        let moving: [f32; 2] =
            std::array::from_fn(|axis| raw[axis] + if near[axis] { 0. } else { raw[axis + 2] });
        let mut closest = [self.distance + 0.0001; 2];
        let mut closest_aspect = self.distance + 0.0001;
        for axis in 0..2 {
            for target in &self.targets[axis] {
                let distance = (target.position - moving[axis]).abs() * self.size[axis];
                if distance > self.distance {
                    continue;
                }
                let extent = if near[axis] {
                    anchor[axis] - target.position
                } else {
                    target.position - anchor[axis]
                };
                if !(0.005..=4.).contains(&extent) {
                    continue;
                }
                if aspect {
                    // One anchor determines a proportional resize. Never snap
                    // both axes independently and distort the item.
                    let factor = extent / original[axis + 2];
                    let dims = [original[2] * factor, original[3] * factor];
                    if dims
                        .iter()
                        .any(|n| !n.is_finite() || !(0.005..=4.).contains(n))
                    {
                        continue;
                    }
                    let candidate: [f32; 4] = [
                        anchor[0] - if near[0] { dims[0] } else { 0. },
                        anchor[1] - if near[1] { dims[1] } else { 0. },
                        dims[0],
                        dims[1],
                    ];
                    let other = 1 - axis;
                    let other_moving = candidate[other]
                        + if near[other] {
                            0.
                        } else {
                            candidate[other + 2]
                        };
                    let other_delta = (other_moving - moving[other]) * self.size[other];
                    let travel = distance.hypot(other_delta);
                    if travel < closest_aspect {
                        closest_aspect = travel;
                        result.rect = candidate;
                        result.guides = [None, None];
                        result.guides[axis] = Some(target.position);
                    }
                } else if distance < closest[axis] {
                    closest[axis] = distance;
                    result.rect[axis] = if near[axis] {
                        target.position
                    } else {
                        anchor[axis]
                    };
                    result.rect[axis + 2] = extent;
                    result.guides[axis] = Some(target.position);
                }
            }
        }
        // A proportional resize can also land exactly on another alignment.
        if aspect && result.guides.iter().any(Option::is_some) {
            for (axis, &is_near) in near.iter().enumerate() {
                if result.guides[axis].is_some() {
                    continue;
                }
                let position = result.rect[axis] + if is_near { 0. } else { result.rect[axis + 2] };
                result.guides[axis] = self.targets[axis]
                    .iter()
                    .find(|t| (t.position - position).abs() * self.size[axis] < 0.1)
                    .map(|t| t.position);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auroracast_core::{Layer, Project};
    fn scene() -> Scene {
        let mut scene = Project::default().scenes.remove(0);
        scene.layers = vec![Layer { crop: [0.;4], group: 0, frame: Default::default(),
            source: 1,
            rect: [0.2, 0.2, 0.2, 0.2],
            visible: true,
            locked: false,
            opacity: 1.,
        }];
        scene
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.00001, "{a} != {b}");
    }
    #[test]
    fn edges_center_threshold_and_preview_zoom() {
        let scene = scene();
        for width in [400., 1200.] {
            let s = Snapper::new(&scene, 0, [width, width / 2.], SnapSettings::default());
            let r = s.apply(
                [7. / width, 0.2, 0.2, 0.2],
                scene.layers[0].rect,
                None,
                true,
            );
            close(r.rect[0], 0.);
            assert_eq!(r.guides[0], Some(0.));
            let r = s.apply(
                [9. / width, 0.2, 0.2, 0.2],
                scene.layers[0].rect,
                None,
                true,
            );
            close(r.rect[0], 9. / width);
            assert_eq!(r.guides[0], None);
            let r = s.apply(
                [0.4 + 3. / width, 0.4, 0.2, 0.2],
                scene.layers[0].rect,
                None,
                true,
            );
            close(r.rect[0], 0.4);
            close(r.rect[1], 0.4);
        }
    }
    #[test]
    fn adjacent_sources_locked_targets_and_hidden_exclusion() {
        let mut scene = scene();
        scene.layers.push(Layer { crop: [0.;4], group: 0, frame: Default::default(),
            source: 2,
            rect: [0.61, 0.2, 0.2, 0.2],
            visible: true,
            locked: true,
            opacity: 1.,
        });
        let settings = SnapSettings {
            canvas_edges: false,
            canvas_center: false,
            ..Default::default()
        };
        let raw = [0.404, 0.204, 0.2, 0.2];
        let r = Snapper::new(&scene, 0, [1000., 600.], settings).apply(raw, raw, None, false);
        close(r.rect[0], 0.41);
        close(r.rect[1], 0.2);
        assert_eq!(r.guides[0], Some(0.61));
        scene.layers[1].visible = false;
        let r = Snapper::new(&scene, 0, [1000., 600.], settings).apply(raw, raw, None, false);
        assert_eq!(r.rect, raw);
        assert_eq!(r.guides, [None, None]);
    }
    #[test]
    fn self_exclusion_disabled_and_no_accumulation() {
        let scene = scene();
        let raw = [0.203, 0.202, 0.2, 0.2];
        let sources = SnapSettings {
            canvas_edges: false,
            canvas_center: false,
            ..Default::default()
        };
        assert_eq!(
            Snapper::new(&scene, 0, [1000., 600.], sources)
                .apply(raw, raw, None, false)
                .rect,
            raw
        );
        let disabled = SnapSettings {
            enabled: false,
            ..Default::default()
        };
        let raw = [0.003, 0.007, 0.2, 0.2];
        assert_eq!(
            Snapper::new(&scene, 0, [1000., 600.], disabled)
                .apply(raw, raw, None, false)
                .rect,
            raw
        );
        let s = Snapper::new(&scene, 0, [1000., 600.], SnapSettings::default());
        for x in [0.003, 0.006, 0.009, 0.003] {
            let r = s.apply([x, 0.2, 0.2, 0.2], raw, None, false);
            close(r.rect[0], if x <= 0.008 { 0. } else { x });
        }
    }
    #[test]
    fn every_resize_corner_keeps_anchor_and_aspect() {
        let scene = scene();
        let original = [0.2, 0.2, 0.6, 0.6];
        let s = Snapper::new(&scene, 0, [500., 500.], SnapSettings::default());
        for corner in 0..4 {
            let left = corner == 0 || corner == 3;
            let top = corner < 2;
            let raw = [
                if left { 0.01 } else { 0.2 },
                if top { 0.01 } else { 0.2 },
                0.79,
                0.79,
            ];
            let r = s.apply(raw, original, Some(corner), true);
            close(r.rect[2], 0.8);
            close(r.rect[3], 0.8);
            close(r.rect[0], if left { 0. } else { 0.2 });
            close(r.rect[1], if top { 0. } else { 0.2 });
            assert!(r.guides.iter().all(Option::is_some));
        }
    }
    #[test]
    fn free_resize_snaps_both_axes_and_cannot_flip() {
        let scene = scene();
        let s = Snapper::new(&scene, 0, [1000., 600.], SnapSettings::default());
        let r = s.apply(
            [0.2, 0.2, 0.797, 0.796],
            [0.2, 0.2, 0.6, 0.4],
            Some(2),
            false,
        );
        close(r.rect[2], 0.8);
        close(r.rect[3], 0.8);
        let r = s.apply(
            [0.499, 0.2, 0.005, 0.2],
            [0.499, 0.2, 0.2, 0.2],
            Some(2),
            false,
        );
        assert!(r.rect[2] >= 0.005);
    }
}
