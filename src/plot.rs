use crate::storage::{CodexSample, UsageSample};
use gtk::prelude::*;
use plotters::prelude::*;
use plotters_cairo::CairoBackend;
use std::{cell::RefCell, rc::Rc};

const MAX_POINTS: usize = 120;

fn recent_slice<T>(v: &[T]) -> &[T] {
    if v.len() > MAX_POINTS {
        &v[v.len() - MAX_POINTS..]
    } else {
        v
    }
}

pub fn make_usage_plot(history: Rc<RefCell<Vec<UsageSample>>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(420);
    area.set_content_height(120);
    area.set_hexpand(true);

    let history_for_draw = history.clone();
    area.set_draw_func(move |_area, cr, width, height| {
        let backend: CairoBackend<'_> = match CairoBackend::new(cr, (width as u32, height as u32)) {
            Ok(b) => b,
            Err(_) => return,
        };
        let root = backend.into_drawing_area();
        let _ = root.fill(&RGBAColor(0, 0, 0, 0.0));

        let hist = history_for_draw.borrow();
        let data = recent_slice(&hist);
        if data.len() < 2 {
            return;
        }

        let max_y = data
            .iter()
            .map(|s| s.session_pct.max(s.weekly_pct))
            .fold(1.0_f64, f64::max)
            .max(100.0);
        let len = data.len() as i32;

        let mut chart = match ChartBuilder::on(&root)
            .margin(6)
            .x_label_area_size(10)
            .y_label_area_size(28)
            .build_cartesian_2d(0..len, 0.0..max_y)
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let _ = chart
            .configure_mesh()
            .disable_mesh()
            .x_labels(0)
            .y_labels(4)
            .y_label_style(TextStyle::from(("Sans", 9).into_font()).with_color(&WHITE.mix(0.7)))
            .draw();

        let _ = chart.draw_series(LineSeries::new(
            data.iter().enumerate().map(|(i, s)| (i as i32, s.session_pct)),
            &Palette99::pick(0).mix(0.85),
        ));
        let _ = chart.draw_series(LineSeries::new(
            data.iter().enumerate().map(|(i, s)| (i as i32, s.weekly_pct)),
            &Palette99::pick(2).mix(0.85),
        ));
    });

    area
}

pub fn make_codex_plot(history: Rc<RefCell<Vec<CodexSample>>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(420);
    area.set_content_height(120);
    area.set_hexpand(true);

    let history_for_draw = history.clone();
    area.set_draw_func(move |_area, cr, width, height| {
        let backend: CairoBackend<'_> = match CairoBackend::new(cr, (width as u32, height as u32)) {
            Ok(b) => b,
            Err(_) => return,
        };
        let root = backend.into_drawing_area();
        let _ = root.fill(&RGBAColor(0, 0, 0, 0.0));

        let hist = history_for_draw.borrow();
        let data = recent_slice(&hist);
        if data.len() < 2 {
            return;
        }

        let max_y = data
            .iter()
            .map(|s| s.primary_pct.max(s.secondary_pct) as f64)
            .fold(1.0_f64, f64::max)
            .max(100.0);
        let len = data.len() as i32;

        let mut chart = match ChartBuilder::on(&root)
            .margin(6)
            .x_label_area_size(10)
            .y_label_area_size(28)
            .build_cartesian_2d(0..len, 0.0..max_y)
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let _ = chart
            .configure_mesh()
            .disable_mesh()
            .x_labels(0)
            .y_labels(4)
            .y_label_style(TextStyle::from(("Sans", 9).into_font()).with_color(&WHITE.mix(0.7)))
            .draw();

        let _ = chart.draw_series(LineSeries::new(
            data.iter().enumerate().map(|(i, s)| (i as i32, s.primary_pct as f64)),
            &Palette99::pick(4).mix(0.85),
        ));
        let _ = chart.draw_series(LineSeries::new(
            data.iter().enumerate().map(|(i, s)| (i as i32, s.secondary_pct as f64)),
            &Palette99::pick(8).mix(0.85),
        ));
    });

    area
}
