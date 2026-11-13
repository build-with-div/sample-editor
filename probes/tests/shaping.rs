//! Probe: the cost of div-text's shaping primitives, which layout calls per
//! inline text item today. It separates shaping from the rest of layout's
//! per-box cost (design.md feature 16) and sizes what canvas `measureText`
//! (feature 18) would pay per call.
//!
//! This uses div-text directly, which is engine-internal and not Div's
//! public API; app code in this sample does not.
//!
//! `cargo test -p editor-probes --test shaping -- --nocapture --test-threads=1`

use div_style::{FontFamilyList, GenericFontFamily};
use div_text::{Font, FontDescription, TextDirection, TextShaper};
use editor_probes::{size, source_line, timed};

fn per_call_us(total: std::time::Duration, n: usize) -> String {
    format!("{:.1} us", total.as_secs_f64() * 1e6 / n as f64)
}

fn monospace() -> FontDescription {
    FontDescription::with_family_and_size(
        FontFamilyList::generic(GenericFontFamily::Monospace),
        13.0,
    )
}

#[test]
fn shaping_primitive_cost() {
    let n = size(200, 2_000);
    // Warm the system collection and font instances.
    let font = Font::new(monospace());
    let shaper = TextShaper::new();
    let line = source_line(1);
    let _ = shaper.shape(&line, &font, TextDirection::Ltr);

    let (_, new_shaper) = timed(|| {
        for _ in 0..n {
            std::hint::black_box(TextShaper::new());
        }
    });
    let (_, new_font) = timed(|| {
        for _ in 0..n {
            std::hint::black_box(Font::new(monospace()));
        }
    });
    let (_, shape_token) = timed(|| {
        for _ in 0..n {
            std::hint::black_box(shaper.shape("total", &font, TextDirection::Ltr));
        }
    });
    let (results, shape_line) = timed(|| {
        (0..n)
            .map(|i| shaper.shape(&source_line(i), &font, TextDirection::Ltr))
            .collect::<Vec<_>>()
    });
    let result = &results[1];
    let chars = line.chars().count();
    let (_, caret) = timed(|| {
        for i in 0..n {
            std::hint::black_box(result.x_position_for_offset(i % chars));
        }
    });
    let (_, hit) = timed(|| {
        for i in 0..n {
            std::hint::black_box(result.offset_for_x_position((i % 300) as f32));
        }
    });
    println!(
        "per call over {n}: TextShaper::new {}, Font::new (monospace 13px) {}, \
         shape a 5-char token {}, shape a ~45-char source line {}, \
         x_position_for_offset {}, offset_for_x_position {}",
        per_call_us(new_shaper, n),
        per_call_us(new_font, n),
        per_call_us(shape_token, n),
        per_call_us(shape_line, n),
        per_call_us(caret, n),
        per_call_us(hit, n),
    );
    // Monospace advance, and ligature-free code: width = chars * advance.
    let advance = result.width / chars as f32;
    println!(
        "line 1: {chars} chars, width {} px, advance {advance} px",
        result.width
    );
    assert!(result.x_position_for_offset(chars) > 0.0);
}
