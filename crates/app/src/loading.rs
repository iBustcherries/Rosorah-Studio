// The loading screen covers the studio while slow start-up work finishes. The studio keeps
// drawing underneath, so its panels and textures are ready when the cover fades away.
struct Loading {
    started: Instant,
    /// Fonts scanned and GStreamer started, on a background thread.
    preloaded: std::sync::Arc<std::sync::atomic::AtomicBool>,
    fading: Option<Instant>,
}
const LOADING_MIN: Duration = Duration::from_millis(800);
const LOADING_MAX: Duration = Duration::from_secs(6);
const LOADING_FADE: Duration = Duration::from_millis(350);
impl Loading {
    fn start() -> Self {
        let preloaded = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = preloaded.clone();
        std::thread::spawn(move || {
            crate::fonts::installed();
            let _ = gst::init();
            done.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        Self { started: Instant::now(), preloaded, fading: None }
    }
}
impl Studio {
    /// What start-up is still waiting for, or None when everything is ready.
    fn loading_step(&self) -> Option<&'static str> {
        let loading = self.loading.as_ref()?;
        let elapsed = loading.started.elapsed();
        if !loading.preloaded.load(std::sync::atomic::Ordering::Relaxed) {
            return Some("Loading fonts and media engine…");
        }
        if self.audio.view().devices.is_empty() && elapsed < Duration::from_secs(3) {
            return Some("Finding audio devices…");
        }
        if self.accounts.iter().chain(std::iter::once(&self.kick_account)).any(|a| a.busy) {
            return Some("Connecting accounts…");
        }
        (elapsed < LOADING_MIN).then_some("Getting your studio ready…")
    }
    fn loading_screen(&mut self, ctx: &egui::Context) {
        let Some(loading) = &self.loading else { return };
        let elapsed = loading.started.elapsed();
        let step = if elapsed >= LOADING_MAX { None } else { self.loading_step() };
        let loading = self.loading.as_mut().unwrap();
        if step.is_none() && loading.fading.is_none() {
            loading.fading = Some(Instant::now());
        }
        let opacity = loading.fading.map_or(1., |t| 1. - t.elapsed().as_secs_f32() / LOADING_FADE.as_secs_f32());
        if opacity <= 0. {
            self.loading = None;
            return;
        }
        let base = bg();
        let cover = Color32::from_rgb(base.r(), base.g(), base.b()).gamma_multiply(opacity);
        let accent = self.accent().gamma_multiply(opacity);
        let text = crate::appearance::current().text.gamma_multiply(opacity);
        let progress = (elapsed.as_secs_f32() / LOADING_MAX.as_secs_f32()).min(1.);
        egui::Area::new(egui::Id::new("loading-screen"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::Pos2::ZERO)
            .interactable(true)
            .show(ctx, |ui| {
                let rect = ctx.content_rect();
                // Blocks clicks on the studio underneath until it is ready.
                ui.allocate_rect(rect, egui::Sense::click_and_drag());
                let painter = ui.painter();
                painter.rect_filled(rect, 0., cover);
                let center = rect.center();
                painter.text(center - Vec2::new(0., 34.), egui::Align2::CENTER_CENTER, "Rosorah Studio", egui::FontId::proportional(40.), text);
                painter.text(center + Vec2::new(0., 6.), egui::Align2::CENTER_CENTER, step.unwrap_or("Ready"), egui::FontId::proportional(15.), text.gamma_multiply(0.7));
                let bar = egui::Rect::from_center_size(center + Vec2::new(0., 40.), Vec2::new(220., 4.));
                painter.rect_filled(bar, 2., text.gamma_multiply(0.15));
                let done = if step.is_none() { 1. } else { 0.15 + 0.75 * progress.sqrt() };
                painter.rect_filled(egui::Rect::from_min_size(bar.min, Vec2::new(bar.width() * done, bar.height())), 2., accent);
                painter.text(rect.center_bottom() - Vec2::new(0., 28.), egui::Align2::CENTER_CENTER, concat!("Version ", env!("CARGO_PKG_VERSION")), egui::FontId::proportional(12.), text.gamma_multiply(0.45));
            });
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
