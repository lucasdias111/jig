//! How each view is arranged, kept in `.jig/map/places.json`, and Tidy.

use super::*;

impl MapView {
    /// Where each of the Plan's boxes is now, by key.
    pub(super) fn positions(&self) -> HashMap<String, (f32, f32)> {
        self.plan
            .nodes
            .iter()
            .zip(&self.placed.nodes)
            .map(|(node, rect)| (node.key.clone(), (rect.x, rect.y)))
            .collect()
    }

    /// The view showing, as its arrangement is kept.
    pub(super) fn view_key(&self) -> String {
        match &self.level {
            Level::Site => "site".into(),
            Level::Domain => "domain".into(),
            Level::Plan(Focus::Module(place)) => format!("plan:{place}"),
            Level::Plan(Focus::Function(symbol)) => {
                format!("plan:{}#{}", symbol.file, symbol.title())
            }
        }
    }

    /// The view's arrangement as it was kept: its places and hops opened,
    /// and where it was scrolled and zoomed to, if it was.
    pub(super) fn load_view(&mut self) -> Option<Camera> {
        self.places.clear();
        self.opened = Opened::default();
        let saved =
            places::read_json(&places::file(&self.root)).map(|all| all[self.view_key()].clone())?;
        if let Some(Value::Object(places)) = saved.get("places") {
            for (key, place) in places {
                if let (Some(x), Some(y)) = (place[0].as_f64(), place[1].as_f64()) {
                    self.places.insert(key.clone(), (x as f32, y as f32));
                }
            }
        }
        if let Some(opened) = saved.get("opened") {
            self.opened = serde_json::from_value(opened.clone()).unwrap_or_default();
        }
        let viewport = saved.get("viewport")?;
        Some(Camera {
            x: viewport[0].as_f64()? as f32,
            y: viewport[1].as_f64()? as f32,
            zoom: viewport[2].as_f64()? as f32,
        })
    }

    /// Keep how the view showing is arranged.
    pub fn save_view(&self) {
        if self.project.is_none() {
            return;
        }
        let places: serde_json::Map<String, Value> = self
            .places
            .iter()
            .map(|(key, (x, y))| (key.clone(), json!([x, y])))
            .collect();
        let file = places::file(&self.root);
        let mut all = match places::read_json(&file) {
            Some(Value::Object(all)) => all,
            _ => serde_json::Map::new(),
        };
        let camera = self.camera();
        all.insert(
            self.view_key(),
            json!({
                "places": places,
                "opened": self.opened,
                "viewport": [camera.x, camera.y, camera.zoom],
            }),
        );
        places::write_json(&file, &Value::Object(all));
        places::keep_out_of_git(&self.root);
    }

    /// Tidy: the Plan laid out afresh.
    pub fn tidy(&mut self, cx: &mut Context<Self>) {
        self.places.clear();
        self.rebuild(true);
        let to = self.default_camera();
        self.transition = Some(Transition::pan(self.camera(), to));
        self.set_camera(to);
        self.save_view();
        cx.notify();
    }

    /// The layout as drawn now, gliding after a change.
    pub(super) fn drawn_plan(&self, now: Instant) -> Placed {
        let mut placed = self.placed.clone();
        let Some((was, start)) = &self.glide else {
            return placed;
        };
        let t = (now.duration_since(*start).as_secs_f32() / GLIDE).min(1.);
        if t >= 1. {
            return placed;
        }
        let eased = 1. - (1. - t).powi(3);
        for (ix, node) in self.plan.nodes.iter().enumerate() {
            if let Some(&(x, y)) = was.get(&node.key) {
                let rect = &mut placed.nodes[ix];
                rect.x = x + (rect.x - x) * eased;
                rect.y = y + (rect.y - y) * eased;
            }
        }
        placed.refit(&self.plan);
        placed
    }
}
