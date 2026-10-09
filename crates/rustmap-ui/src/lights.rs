//! DMX fixtures: the list, the inspector (patch and fixture type editor) and
//! the console with a fader per channel.

use egui::{Color32, Pos2, Rect, Sense, Stroke};
use rustmap_core::led::{LightId, conflicts_for};
use rustmap_core::{
    ChannelDef, ChannelKind, DmxFixture, DmxFixtureId, DmxType, DmxTypeId, Project, ValueRange,
};

use crate::{UiAction, UiState, ViewData};

const FADER_W: f32 = 38.0;
const FADER_H: f32 = 150.0;
/// Width of the master group.
const MASTER_W: f32 = 120.0;
/// Space between faders.
const FADER_GAP: f32 = 4.0;

/// Outer width of a fixture's console strip (frame included).
fn strip_width(channels: usize) -> f32 {
    (channels as f32 * (FADER_W + FADER_GAP)).max(190.0) + 16.0
}

/// Fader color per channel kind.
fn kind_color(kind: ChannelKind) -> Color32 {
    match kind {
        ChannelKind::Dimmer => Color32::from_gray(200),
        ChannelKind::Red => Color32::from_rgb(225, 60, 60),
        ChannelKind::Green => Color32::from_rgb(60, 200, 90),
        ChannelKind::Blue => Color32::from_rgb(70, 110, 240),
        ChannelKind::White => Color32::from_rgb(235, 235, 220),
        ChannelKind::Amber => Color32::from_rgb(245, 160, 40),
        ChannelKind::Uv => Color32::from_rgb(150, 80, 230),
        ChannelKind::Pan | ChannelKind::Tilt => Color32::from_rgb(60, 180, 200),
        ChannelKind::Strobe => Color32::from_rgb(230, 210, 60),
        _ => Color32::from_rgb(120, 130, 150),
    }
}

/// "U0 · 1–9" for the list and console.
fn address(f: &DmxFixture, footprint: usize) -> String {
    let first = f.first_channel() + 1;
    if footprint <= 1 {
        format!("U{} · {first}", f.patch.universe)
    } else {
        format!("U{} · {first}–{}", f.patch.universe, first + footprint - 1)
    }
}

// ---- list ----

pub(crate) fn list(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    actions: &mut Vec<UiAction>,
) {
    ui.horizontal(|ui| {
        ui.strong("DMX FIXTURES");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button("+", |ui| add_menu(ui, project, state, actions))
                .response
                .on_hover_text("Add a DMX fixture or define a fixture type");
        });
    });
    ui.separator();
    if project.dmx_fixtures.is_empty() {
        ui.weak("None yet. + adds one from a template or your own type.");
    }
    let mut remove = None;
    let ids: Vec<_> = project.dmx_fixtures.iter().map(|f| f.id).collect();
    for id in ids {
        let footprint = project
            .dmx_fixture(id)
            .and_then(|f| project.dmx_type(f.type_id))
            .map_or(0, DmxType::footprint);
        let Some(f) = project.dmx_fixture_mut(id) else {
            continue;
        };
        ui.horizontal(|ui| {
            ui.checkbox(&mut f.enabled, "")
                .on_hover_text("Send this fixture (off: its channels go out as zero)");
            if ui
                .selectable_label(state.selected_dmx == Some(id), &f.name)
                .clicked()
            {
                state.select_dmx(Some(id));
            }
            ui.weak(address(f, footprint));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("🗑")
                    .on_hover_text("Remove fixture")
                    .clicked()
                {
                    remove = Some(id);
                }
            });
        });
    }
    if let Some(id) = remove {
        project.remove_dmx_fixture(id);
    }
    ui.add_space(6.0);
    crate::fixtures::send_settings(ui, project);
}

fn add_menu(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    actions: &mut Vec<UiAction>,
) {
    let add = |project: &mut Project, state: &mut UiState, ty: DmxTypeId| {
        if let Some(id) = project.add_dmx_fixture(ty) {
            state.select_dmx(Some(id));
            state.editor_mode = crate::EditorMode::Console;
        }
    };
    if !project.dmx_types.is_empty() {
        ui.weak("Add a fixture of type");
        let types: Vec<_> = project
            .dmx_types
            .iter()
            .map(|t| (t.id, format!("{} ({} ch)", t.name, t.footprint())))
            .collect();
        for (id, label) in types {
            if ui.button(label).clicked() {
                add(project, state, id);
            }
        }
        ui.separator();
    }
    if ui
        .button("New fixture type…")
        .on_hover_text("Define your own: a name and its channels")
        .clicked()
    {
        let ty = project.add_dmx_type(DmxType {
            id: DmxTypeId(0),
            name: "My fixture".into(),
            channels: vec![ChannelDef::new("Dimmer", ChannelKind::Dimmer)],
        });
        add(project, state, ty);
    }
    ui.menu_button("From template", |ui| {
        for t in DmxType::templates() {
            if ui
                .button(format!("{} ({} ch)", t.name, t.footprint()))
                .clicked()
            {
                let ty = project.add_dmx_type(t);
                add(project, state, ty);
            }
        }
    });
    if ui
        .button("Import fixture types…")
        .on_hover_text("Load types saved from another show (.rmfixture)")
        .clicked()
    {
        actions.push(UiAction::ImportDmxTypes);
    }
}

// ---- inspector ----

pub(crate) fn inspector(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    actions: &mut Vec<UiAction>,
) {
    let Some(id) = state.selected_dmx else { return };
    let conflicts = conflicts_for(project, LightId::Dmx(id));
    let types: Vec<(DmxTypeId, String)> = project
        .dmx_types
        .iter()
        .map(|t| (t.id, t.name.clone()))
        .collect();
    let Some(f) = project.dmx_fixture_mut(id) else {
        return;
    };
    let type_before = f.type_id;
    let mut duplicate = false;
    egui::Grid::new("dmx fixture props")
        .num_columns(2)
        .spacing([8.0, 6.0])
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut f.name);
            ui.end_row();

            ui.label("Send");
            ui.checkbox(&mut f.enabled, "");
            ui.end_row();

            ui.label("Type");
            let current = types
                .iter()
                .find(|(t, _)| *t == f.type_id)
                .map_or("?", |(_, n)| n.as_str())
                .to_owned();
            egui::ComboBox::from_id_salt("dmx type")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (t, name) in &types {
                        ui.selectable_value(&mut f.type_id, *t, name);
                    }
                });
            ui.end_row();
        });
    ui.add_space(8.0);
    crate::fixtures::patch_editor(ui, &mut f.patch, "dmx patch");
    let type_id = f.type_id;
    if f.type_id != type_before {
        project.refit_dmx_type(type_id);
    }

    let Some(f) = project.dmx_fixture(id) else {
        return;
    };
    if let Some(ty) = project.dmx_type(type_id) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.weak(format!(
                "{} channels · {}",
                ty.footprint(),
                address(f, ty.footprint())
            ));
            duplicate = ui
                .button("Duplicate")
                .on_hover_text("Patch another fixture of this type after the last one")
                .clicked();
        });
        if !f.fits(ty) {
            ui.colored_label(
                crate::editor::ERROR,
                "Runs past channel 512; the last channels aren't sent.",
            );
        }
    }
    for c in conflicts {
        ui.colored_label(crate::editor::ERROR, c);
    }
    if duplicate && let Some(new) = project.add_dmx_fixture(type_id) {
        let values = project.dmx_fixture(id).map(|f| f.values.clone());
        if let (Some(values), Some(n)) = (values, project.dmx_fixture_mut(new)) {
            n.values = values;
        }
        state.select_dmx(Some(new));
        return;
    }

    ui.add_space(10.0);
    let users = project
        .dmx_fixtures
        .iter()
        .filter(|f| f.type_id == type_id)
        .count();
    egui::CollapsingHeader::new("Fixture type")
        .default_open(true)
        .show(ui, |ui| {
            type_editor(ui, project, type_id, users, actions);
        });
    project.refit_dmx_type(type_id);
}

fn type_editor(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: DmxTypeId,
    users: usize,
    actions: &mut Vec<UiAction>,
) {
    let Some(ty) = project.dmx_type_mut(id) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut ty.name);
    });
    if users > 1 {
        ui.weak(format!(
            "Changes apply to all {users} fixtures of this type."
        ));
    }
    ui.add_space(4.0);

    let mut action: Option<(usize, &str)> = None;
    egui::Grid::new("type channels")
        .num_columns(6)
        .spacing([4.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            ui.weak("#");
            ui.weak("Label");
            ui.weak("Kind");
            ui.weak("16-bit");
            ui.weak("Default");
            ui.weak("");
            ui.end_row();
            let mut offset = 1;
            let n = ty.channels.len();
            for (i, c) in ty.channels.iter_mut().enumerate() {
                ui.weak(if c.fine {
                    format!("{offset}–{}", offset + 1)
                } else {
                    offset.to_string()
                });
                offset += c.width();
                ui.add_sized([90.0, 20.0], egui::TextEdit::singleline(&mut c.name));
                egui::ComboBox::from_id_salt(("kind", i))
                    .selected_text(c.kind.label())
                    .width(80.0)
                    .show_ui(ui, |ui| {
                        for k in ChannelKind::ALL {
                            ui.selectable_value(&mut c.kind, k, k.label());
                        }
                    })
                    .response
                    .on_hover_text(
                        "Sets the fader color and how master, blackout and Locate treat it",
                    );
                if ui
                    .checkbox(&mut c.fine, "")
                    .on_hover_text("Two channels: coarse, then fine (e.g. pan/tilt)")
                    .changed()
                {
                    c.default = if c.fine {
                        c.default << 8
                    } else {
                        c.default >> 8
                    };
                }
                let max = c.max();
                ui.add(egui::DragValue::new(&mut c.default).range(0..=max));
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(i > 0, egui::Button::new("⬆").small())
                        .clicked()
                    {
                        action = Some((i, "up"));
                    }
                    if ui
                        .add_enabled(i + 1 < n, egui::Button::new("⬇").small())
                        .clicked()
                    {
                        action = Some((i, "down"));
                    }
                    if ui
                        .add_enabled(n > 1, egui::Button::new("🗙").small())
                        .on_hover_text("Remove channel")
                        .clicked()
                    {
                        action = Some((i, "remove"));
                    }
                });
                ui.end_row();
            }
        });
    match action {
        Some((i, "up")) => ty.channels.swap(i, i - 1),
        Some((i, "down")) => ty.channels.swap(i, i + 1),
        Some((i, "remove")) => {
            ty.channels.remove(i);
        }
        _ => {}
    }
    if ui.button("+ Channel").clicked() {
        let n = ty.channels.len() + 1;
        ty.channels
            .push(ChannelDef::new(&format!("Channel {n}"), ChannelKind::Other));
    }

    ui.add_space(6.0);
    ui.weak("Value labels: name ranges of a channel, e.g. Gobo 1 = 8–15. They show under the fader and as quick picks.");
    for (i, c) in ty.channels.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("{} ({} labels)", c.name, c.ranges.len()))
            .id_salt(("ranges", i))
            .show(ui, |ui| ranges_editor(ui, c));
    }

    ui.add_space(8.0);
    let mut delete = false;
    ui.horizontal(|ui| {
        if ui
            .button("Export type…")
            .on_hover_text("Save this type to a file to reuse it in other shows")
            .clicked()
        {
            actions.push(UiAction::ExportDmxType(id));
        }
        delete = ui
            .button(if users > 0 {
                format!("Delete type and its {users} fixture(s)")
            } else {
                "Delete type".to_owned()
            })
            .clicked();
    });
    if delete {
        project.remove_dmx_type(id);
    }
}

fn ranges_editor(ui: &mut egui::Ui, c: &mut ChannelDef) {
    let mut remove = None;
    for (i, r) in c.ranges.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut r.from).range(0..=255));
            ui.label("–");
            ui.add(egui::DragValue::new(&mut r.to).range(0..=255));
            ui.add(egui::TextEdit::singleline(&mut r.label).desired_width(110.0));
            if ui.small_button("🗙").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        c.ranges.remove(i);
    }
    if ui.small_button("+ Label").clicked() {
        let from = c.ranges.last().map_or(0, |r| r.to.saturating_add(1));
        c.ranges.push(ValueRange {
            from,
            to: from.saturating_add(9),
            label: format!("Label {}", c.ranges.len() + 1),
        });
    }
}

// ---- console ----

pub(crate) fn console(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    // Cues in a bottom panel: half the console when open, one row of
    // buttons when collapsed. Faders fill the rest.
    let half = (ui.available_height() * 0.5).max(120.0);
    let panel = if state.cues_open {
        egui::Panel::bottom("console_cues_open")
            .resizable(true)
            .default_size(half)
            .min_size(90.0)
    } else {
        egui::Panel::bottom("console_cues_closed").resizable(false)
    };
    panel
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(0, 6)))
        .show(ui, |ui| cues(ui, project, state, view, actions));

    egui::ScrollArea::vertical()
        .id_salt("console_faders")
        .auto_shrink(false)
        .show(ui, |ui| {
        groups(ui, project);
        ui.add_space(6.0);
        // egui can't wrap nested groups by itself, so build the rows from
        // each strip's known width.
        const GAP: f32 = 10.0;
        let avail = ui.available_width();
        let mut rows: Vec<Vec<Option<DmxFixtureId>>> = vec![vec![None]]; // None = masters
        let mut x = MASTER_W + GAP;
        for f in &project.dmx_fixtures {
            let channels = project.dmx_type(f.type_id).map_or(1, |t| t.channels.len());
            let w = strip_width(channels);
            if x + w > avail && x > 0.0 {
                rows.push(Vec::new());
                x = 0.0;
            }
            rows.last_mut().expect("a row").push(Some(f.id));
            x += w + GAP;
        }
        for row in rows {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
                for item in row {
                    match item {
                        None => masters(ui, project),
                        Some(id) => fixture_strip(ui, project, state, id),
                    }
                }
            });
            ui.add_space(GAP);
        }
        if project.dmx_fixtures.is_empty() {
            ui.add_space(12.0);
            ui.weak("No DMX fixtures yet. Add one under DMX fixtures (+) on the left: from a template, or define your own type with its channels.");
        }
    });
}

/// Grand master and blackout.
fn masters(ui: &mut egui::Ui, project: &mut Project) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(MASTER_W - 16.0);
            ui.strong("Master");
            let sending = project.led.send;
            let label = egui::RichText::new(if sending { "Sending" } else { "Send to lights" })
                .color(if sending {
                    Color32::WHITE
                } else {
                    Color32::from_gray(200)
                });
            if ui
                .add_enabled(
                    project.has_lights(),
                    egui::Button::selectable(sending, label)
                        .min_size(egui::vec2(MASTER_W - 16.0, 22.0)),
                )
                .on_hover_text("Send all LED and DMX fixtures over Art-Net / sACN")
                .clicked()
            {
                project.led.send = !sending;
            }
            ui.add(
                egui::DragValue::new(&mut project.led.fps)
                    .range(1.0..=120.0)
                    .max_decimals(0)
                    .suffix(" fps"),
            )
            .on_hover_text("Packets per second; most controllers want 30–44");
            let blackout = project.led.blackout;
            let text = egui::RichText::new("Blackout").color(if blackout {
                Color32::WHITE
            } else {
                Color32::from_gray(200)
            });
            let button = egui::Button::new(text).fill(if blackout {
                Color32::from_rgb(170, 40, 40)
            } else {
                Color32::from_gray(45)
            });
            if ui
                .add_sized([MASTER_W - 16.0, 22.0], button)
                .on_hover_text("Everything dark: LED pixels and DMX intensity channels")
                .clicked()
            {
                project.led.blackout = !blackout;
            }
            let mut v = (project.led.master.clamp(0.0, 1.0) * 255.0).round() as u16;
            let label = format!("{:.0}%", project.led.master * 100.0);
            if fader(ui, &mut v, 255, 255, Color32::from_gray(230), &label) {
                project.led.master = f32::from(v) / 255.0;
            }
            ui.small("Grand master");
        });
    });
}

fn fixture_strip(ui: &mut egui::Ui, project: &mut Project, state: &mut UiState, id: DmxFixtureId) {
    let Some(f) = project.dmx_fixture(id) else {
        return;
    };
    let Some(ty) = project.dmx_type(f.type_id).cloned() else {
        return;
    };
    let selected = state.selected_dmx == Some(id);
    let frame = egui::Frame::group(ui.style()).stroke(if selected {
        Stroke::new(1.5, crate::SELECTION)
    } else {
        ui.style().visuals.widgets.noninteractive.bg_stroke
    });
    let footprint = ty.footprint();
    let Some(f) = project.dmx_fixture_mut(id) else {
        return;
    };
    f.fit_to(&ty);
    let width = strip_width(ty.channels.len()) - 16.0;
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(selected, egui::RichText::new(&f.name).strong())
                    .clicked()
                {
                    state.select_dmx(Some(id));
                }
                ui.weak(address(f, footprint));
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut f.enabled, "On");
                if ui
                    .small_button("Locate")
                    .on_hover_text("Intensity and colors full, everything else at default")
                    .clicked()
                {
                    f.values = DmxFixture::locate_values(&ty);
                }
                if ui
                    .small_button("Reset")
                    .on_hover_text("All channels to their defaults")
                    .clicked()
                {
                    f.values = ty.defaults();
                }
            });
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = FADER_GAP;
                for (i, def) in ty.channels.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(FADER_W);
                        let mut v = f.values[i];
                        let text = if def.fine {
                            format!("{:.1}%", f32::from(v) / 655.35)
                        } else {
                            v.to_string()
                        };
                        if fader(
                            ui,
                            &mut v,
                            def.max(),
                            def.default,
                            kind_color(def.kind),
                            &text,
                        ) {
                            f.values[i] = v;
                        }
                        ui.add(egui::Label::new(egui::RichText::new(&def.name).small()).truncate())
                            .on_hover_text(format!("{} · {}", def.name, def.kind.label()));
                        if !def.ranges.is_empty() {
                            let current =
                                def.range_at(f.values[i]).map_or("—", |r| r.label.as_str());
                            ui.menu_button(egui::RichText::new(current).small(), |ui| {
                                for r in &def.ranges {
                                    let label = format!("{} ({}–{})", r.label, r.from, r.to);
                                    if ui.button(label).clicked() {
                                        let coarse = u16::from(r.from.min(r.to));
                                        f.values[i] = if def.fine { coarse << 8 } else { coarse };
                                    }
                                }
                            })
                            .response
                            .on_hover_text("Pick a labelled value");
                        }
                    });
                }
            });
        });
    });
}

/// A vertical fader: drag to set (Shift: fine), scroll to step, double-click
/// for the default. Returns true if the value changed.
fn fader(
    ui: &mut egui::Ui,
    value: &mut u16,
    max: u16,
    default: u16,
    color: Color32,
    text: &str,
) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(FADER_W, FADER_H), Sense::click_and_drag());
    let response =
        response.on_hover_text("Drag (Shift: fine) · scroll to step · double-click: default");
    let old = *value;
    let track = Rect::from_min_max(
        Pos2::new(rect.min.x + 4.0, rect.min.y + 18.0),
        Pos2::new(rect.max.x - 4.0, rect.max.y - 2.0),
    );
    let to_value = |y: f32| {
        let t = ((track.max.y - y) / track.height()).clamp(0.0, 1.0);
        (t * f32::from(max)).round() as u16
    };
    let shift = ui.input(|i| i.modifiers.shift);
    if response.dragged() {
        if shift {
            let dy = response.drag_delta().y;
            let step = (f32::from(max) / track.height() / 10.0).max(0.1);
            let v = f32::from(*value) - dy * step;
            *value = v.clamp(0.0, f32::from(max)).round() as u16;
        } else if let Some(p) = response.interact_pointer_pos() {
            *value = to_value(p.y);
        }
    } else if response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        *value = to_value(p.y);
    }
    if response.double_clicked() {
        *value = default.min(max);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let step = if max > 255 { 256.0 } else { 1.0 };
            let v = f32::from(*value) + scroll.signum() * step;
            *value = v.clamp(0.0, f32::from(max)) as u16;
        }
    }

    let painter = ui.painter_at(rect);
    painter.text(
        Pos2::new(rect.center().x, rect.min.y + 1.0),
        egui::Align2::CENTER_TOP,
        text,
        egui::FontId::monospace(11.0),
        Color32::from_gray(220),
    );
    painter.rect_filled(track, 3.0, Color32::from_gray(28));
    let t = f32::from(*value) / f32::from(max.max(1));
    let fill = Rect::from_min_max(
        Pos2::new(track.min.x, track.max.y - track.height() * t),
        track.max,
    );
    painter.rect_filled(fill, 3.0, color.gamma_multiply(0.75));
    painter.rect_stroke(
        track,
        3.0,
        Stroke::new(
            1.0,
            if response.hovered() || response.dragged() {
                color
            } else {
                Color32::from_gray(60)
            },
        ),
        egui::StrokeKind::Inside,
    );
    *value != old
}

// ---- cues ----

/// Cue list: record the current look, go to it, fade between looks. The
/// first row (record, back / current / next) stays visible when collapsed.
fn cues(
    ui: &mut egui::Ui,
    project: &mut Project,
    state: &mut UiState,
    view: &ViewData<'_>,
    actions: &mut Vec<UiAction>,
) {
    let current = view.dmx.current_cue;
    let any = !project.cues.is_empty();
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), Sense::click());
        let openness = if state.cues_open { 1.0 } else { 0.0 };
        egui::collapsing_header::paint_default_icon(
            &mut ui.new_child(egui::UiBuilder::new().max_rect(rect)),
            openness,
            &response,
        );
        let title =
            ui.add(egui::Label::new(egui::RichText::new("CUES").strong()).sense(Sense::click()));
        if response.clicked() || title.clicked() {
            state.cues_open = !state.cues_open;
        }
        title.on_hover_text(if state.cues_open {
            "Collapse the cue list (buttons stay)"
        } else {
            "Show the cue list"
        });
        ui.separator();
        if ui
            .button("Record all")
            .on_hover_text("Store every fixture's faders as a new cue")
            .clicked()
        {
            project.record_cue(None);
        }
        let selected = state.selected_dmx;
        if ui
            .add_enabled(selected.is_some(), egui::Button::new("Record selected"))
            .on_hover_text("Store only the selected fixture")
            .on_disabled_hover_text("Select a DMX fixture first")
            .clicked()
            && let Some(id) = selected
        {
            project.record_cue(Some(&[id]));
        }
        ui.separator();
        if ui
            .add_enabled(any, egui::Button::new("Back"))
            .on_hover_text("Previous cue")
            .clicked()
        {
            actions.push(UiAction::StepCue(false));
        }
        if ui
            .add_enabled(current.is_some(), egui::Button::new("Current"))
            .on_hover_text("Apply the current cue again (after moving faders)")
            .on_disabled_hover_text("Go to a cue first")
            .clicked()
        {
            actions.push(UiAction::CurrentCue);
        }
        if ui
            .add_enabled(any, egui::Button::new("Next"))
            .on_hover_text("Next cue")
            .clicked()
        {
            actions.push(UiAction::StepCue(true));
        }
        ui.separator();
        let live = current.and_then(|id| {
            let n = project.cues.iter().position(|c| c.id == id)?;
            Some(format!("{} {}", n + 1, project.cues[n].name))
        });
        match (view.dmx.fade, live) {
            (Some(t), Some(name)) => {
                ui.add(
                    egui::ProgressBar::new(t)
                        .desired_width(160.0)
                        .text(format!("{name}  {:.0}%", t * 100.0)),
                );
            }
            (None, Some(name)) => {
                ui.label(egui::RichText::new(name).color(crate::SELECTION));
            }
            _ => {
                ui.weak("no cue");
            }
        }
    });
    if !state.cues_open {
        return;
    }
    osc_row(ui, project, view);
    if !any {
        ui.weak("No cues yet. Set the faders the way you want, then Record.");
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("cue_list")
        .auto_shrink(false)
        .show(ui, |ui| {
            let mut remove = None;
            let mut update = None;
            let ids: Vec<_> = project.cues.iter().map(|c| c.id).collect();
            for (n, id) in ids.into_iter().enumerate() {
                let Some(cue) = project.cue_mut(id) else {
                    continue;
                };
                ui.horizontal(|ui| {
                    let live = current == Some(id);
                    ui.add_sized(
                        [22.0, 18.0],
                        egui::Label::new(egui::RichText::new(format!("{}", n + 1)).monospace()),
                    )
                    .on_hover_text(format!("OSC: /rustmap/cue/go {}", n + 1));
                    if ui
                        .add(egui::Button::new("Go").fill(if live {
                            crate::SELECTION.gamma_multiply(0.5)
                        } else {
                            Color32::TRANSPARENT
                        }))
                        .clicked()
                    {
                        actions.push(UiAction::GoCue(id));
                    }
                    ui.add(egui::TextEdit::singleline(&mut cue.name).desired_width(140.0));
                    ui.add(
                        egui::DragValue::new(&mut cue.fade)
                            .range(0.0..=600.0)
                            .speed(0.05)
                            .suffix(" s")
                            .max_decimals(1),
                    )
                    .on_hover_text(
                        "Fade time (colors, dimmer, pan, tilt, zoom and focus blend; the rest jump)",
                    );
                    ui.weak(format!("{} fixtures", cue.values.len()));
                    if ui
                        .small_button("Update")
                        .on_hover_text("Overwrite this cue with the current faders (same fixtures)")
                        .clicked()
                    {
                        update = Some(id);
                    }
                    if ui.small_button("🗑").on_hover_text("Delete cue").clicked() {
                        remove = Some(id);
                    }
                });
            }
            if let Some(id) = update {
                let only: Vec<_> = project
                    .cue(id)
                    .map(|c| c.values.keys().copied().collect())
                    .unwrap_or_default();
                project.update_cue(id, Some(&only));
            }
            if let Some(id) = remove {
                project.remove_cue(id);
            }
        });
}

// ---- groups ----

/// Groups: one set of faders for several fixtures.
fn groups(ui: &mut egui::Ui, project: &mut Project) {
    egui::CollapsingHeader::new(egui::RichText::new("GROUPS").strong())
        .default_open(true)
        .show(ui, |ui| {
            if ui
                .button("+ Group")
                .on_hover_text("Control several fixtures with one set of faders")
                .clicked()
            {
                project.add_dmx_group();
            }
            if project.dmx_groups.is_empty() {
                ui.weak("No groups yet.");
            }
            let mut remove = None;
            let ids: Vec<_> = project.dmx_groups.iter().map(|g| g.id).collect();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                for id in ids {
                    if group_strip(ui, project, id) {
                        remove = Some(id);
                    }
                }
            });
            if let Some(id) = remove {
                project.remove_dmx_group(id);
            }
        });
}

/// One group's strip. Returns true if it should be deleted.
fn group_strip(ui: &mut egui::Ui, project: &mut Project, id: rustmap_core::DmxGroupId) -> bool {
    let Some(group) = project.dmx_group(id).cloned() else {
        return false;
    };
    let controls = rustmap_core::group_controls(project, &group);
    let mut delete = false;
    let mut name = group.name.clone();
    let mut members = group.fixtures.clone();
    let mut changed: Option<(usize, f32)> = None;
    let width = strip_width(controls.len().max(2)) - 16.0;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(90.0));
                if ui.small_button("🗑").on_hover_text("Delete group").clicked() {
                    delete = true;
                }
            });
            ui.menu_button(format!("Members ({})", members.len()), |ui| {
                for f in &project.dmx_fixtures {
                    let mut on = members.contains(&f.id);
                    if ui.checkbox(&mut on, &f.name).changed() {
                        if on {
                            members.push(f.id);
                        } else {
                            members.retain(|m| *m != f.id);
                        }
                    }
                }
                if project.dmx_fixtures.is_empty() {
                    ui.weak("No fixtures");
                }
            });
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = FADER_GAP;
                for (n, c) in controls.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(FADER_W);
                        let v0 = (c.value(project) * 1000.0).round() as u16;
                        let mut v = v0;
                        let text = format!("{:.0}%", f32::from(v) / 10.0);
                        if fader(ui, &mut v, 1000, 0, kind_color(c.kind), &text) {
                            changed = Some((n, f32::from(v) / 1000.0));
                        }
                        ui.add(egui::Label::new(egui::RichText::new(&c.label).small()).truncate())
                            .on_hover_text(format!(
                                "{} · {} fixture channels",
                                c.label,
                                c.members.len()
                            ));
                    });
                }
            });
        });
    });
    if let Some((n, v)) = changed {
        controls[n].set(project, v);
    }
    if let Some(g) = project.dmx_group_mut(id) {
        g.name = name;
        g.fixtures = members;
    }
    delete
}

/// Addresses RUSTMAP understands, for the hover help.
const OSC_HELP: &str = "\
/rustmap/cue/go <number or name>
/rustmap/cue/go/<number>
/rustmap/cue/next   (or /rustmap/go)
/rustmap/cue/back
/rustmap/cue/current
/rustmap/blackout [0|1]   (no argument: toggle)
/rustmap/master <0..1>

Cue numbers are the positions in this list (from 1).
A 0 argument on a trigger is a button release and is ignored.";

/// OSC listen switch, port and status.
fn osc_row(ui: &mut egui::Ui, project: &mut Project, view: &ViewData<'_>) {
    let osc = view.osc;
    ui.horizontal(|ui| {
        ui.checkbox(&mut project.osc.listen, "OSC")
            .on_hover_text(format!(
                "Trigger cues from TouchOSC, QLab, Companion…\n\n{OSC_HELP}"
            ));
        ui.label("port");
        ui.add(
            egui::DragValue::new(&mut project.osc.port)
                .range(1..=65535)
                .speed(1),
        );
        if let Some(err) = &osc.error {
            ui.colored_label(ui.visuals().error_fg_color, err);
        } else if let Some(port) = osc.listening {
            ui.weak(format!(
                "listening on UDP {port} · {} received",
                osc.received
            ))
            .on_hover_text(OSC_HELP);
        }
    });
    if project.osc.listen && !osc.recent.is_empty() {
        egui::CollapsingHeader::new(egui::RichText::new("Recent OSC").small())
            .id_salt("osc_recent")
            .show(ui, |ui| {
                for line in &osc.recent {
                    ui.label(egui::RichText::new(line).small().monospace());
                }
            });
    }
}
