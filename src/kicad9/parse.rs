//! Parse s-expression tree into Footprint data.

use super::data::*;
use super::error::{Error, Result};
use super::sexp::{parse as parse_sexp, Sexp};
use std::collections::HashMap;
use std::path::Path;

/// Parse footprint from string.
pub fn parse_footprint(s: &str) -> Result<Footprint> {
    let sexp = parse_sexp(s)?;
    from_sexp(&sexp)
}

/// Read footprint from file.
pub fn read_footprint(path: &Path) -> Result<Footprint> {
    let s = std::fs::read_to_string(path)?;
    parse_footprint(&s)
}

/// Load all .kicad_mod files in a directory.
pub fn load_footprint_dir(dir: &Path) -> Result<HashMap<String, Footprint>> {
    let mut map = HashMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("kicad_mod") {
            if let Ok(fp) = read_footprint(&path) {
                map.insert(fp.name.clone(), fp);
            }
        }
    }
    Ok(map)
}

fn from_sexp(s: &Sexp) -> Result<Footprint> {
    let (_root_name, rest): (&str, &[Sexp]) = match s {
        Sexp::List(n, r) if n == "footprint" || n == "module" => (n.as_str(), r),
        Sexp::List(n, _) => {
            return Err(Error::Parse {
                message: format!("Expected (footprint ...) or (module ...), got ({})", n),
                offset: None,
            })
        }
        Sexp::Atom(_) => {
            return Err(Error::Parse {
                message: "Expected footprint list".into(),
                offset: None,
            })
        }
    };

    let mut fp = Footprint {
        name: rest.first().and_then(|e| e.string()).unwrap_or("").to_string(),
        version: None,
        generator: None,
        layer: "F.Cu".into(),
        description: None,
        tags: None,
        attr: None,
        properties: Vec::new(),
        elements: Vec::new(),
        model: None,
    };

    for e in rest.iter().skip(1) {
        if let Sexp::List(n, args) = e {
            match n.as_str() {
                "version" => fp.version = args.first().and_then(|a| a.string()).map(String::from),
                "generator" => fp.generator = args.first().and_then(|a| a.string()).map(String::from),
                "layer" => fp.layer = args.first().and_then(|a| a.string()).unwrap_or("F.Cu").into(),
                "descr" => fp.description = args.first().and_then(|a| a.string()).map(String::from),
                "tags" => fp.tags = args.first().and_then(|a| a.string()).map(String::from),
                "attr" => fp.attr = args.first().and_then(|a| a.string()).map(String::from),
                "property" => {
                    if let (Some(k), Some(v)) = (
                        args.get(0).and_then(|a| a.string()),
                        args.get(1).and_then(|a| a.string()),
                    ) {
                        fp.properties.push(Property {
                            key: k.to_string(),
                            value: v.to_string(),
                        });
                    }
                }
                "pad" => {
                    if let Ok(pad) = parse_pad(e) {
                        fp.elements.push(Element::Pad(pad));
                    }
                }
                "fp_line" => {
                    if let Ok(line) = parse_fp_line(e) {
                        fp.elements.push(Element::FpLine(line));
                    }
                }
                "fp_rect" => {
                    if let Ok(rect) = parse_fp_rect(e) {
                        fp.elements.push(Element::FpRect(rect));
                    }
                }
                "fp_circle" => {
                    if let Ok(circle) = parse_fp_circle(e) {
                        fp.elements.push(Element::FpCircle(circle));
                    }
                }
                "fp_arc" => {
                    if let Ok(arc) = parse_fp_arc(e) {
                        fp.elements.push(Element::FpArc(arc));
                    }
                }
                "fp_poly" => {
                    if let Ok(poly) = parse_fp_poly(e) {
                        fp.elements.push(Element::FpPoly(poly));
                    }
                }
                "fp_curve" => {
                    if let Ok(curve) = parse_fp_curve(e) {
                        fp.elements.push(Element::FpCurve(curve));
                    }
                }
                "fp_text" => {
                    if let Ok(text) = parse_fp_text(e) {
                        fp.elements.push(Element::FpText(text));
                    }
                }
                "model" => fp.model = parse_model(e).ok(),
                _ => {}
            }
        }
    }

    Ok(fp)
}

fn parse_point(sexp: &Sexp) -> Option<Point> {
    let rest = sexp.list_rest()?;
    let x = rest.get(0)?.string()?.parse().ok()?;
    let y = rest.get(1)?.string()?.parse().ok()?;
    Some(Point::new(x, y))
}

fn parse_at_optional_rot(sexp: &Sexp) -> Point {
    let rest = sexp.list_rest().unwrap_or(&[]);
    let x = rest.get(0).and_then(|a| a.string()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let y = rest.get(1).and_then(|a| a.string()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    Point::new(x, y)
}

fn extract_width_from_stroke(sexp: &Sexp) -> f64 {
    let stroke = sexp.find("stroke");
    if let Some(Sexp::List(_, args)) = stroke {
        for a in args {
            if let Sexp::List(n, rest) = a {
                if n == "width" {
                    return rest.first().and_then(|r| r.string()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                }
            }
        }
    }
    0.0
}

fn parse_pad(sexp: &Sexp) -> Result<Pad> {
    let rest = sexp.list_rest().ok_or_else(|| Error::Parse {
        message: "pad: expected list".into(),
        offset: None,
    })?;

    let name = rest.get(0).and_then(|a| a.string()).unwrap_or("").to_string();
    let ty = rest
        .get(1)
        .and_then(|a| a.string())
        .and_then(|s| match s {
            "smd" => Some(PadType::Smd),
            "thru_hole" => Some(PadType::ThruHole),
            "np_thru_hole" => Some(PadType::NpThruHole),
            "connect" => Some(PadType::Connect),
            _ => Some(PadType::ThruHole),
        })
        .unwrap_or(PadType::ThruHole);
    let shape = rest
        .get(2)
        .and_then(|a| a.string())
        .and_then(|s| match s {
            "circle" => Some(PadShape::Circle),
            "rect" => Some(PadShape::Rect),
            "oval" => Some(PadShape::Oval),
            "trapezoid" => Some(PadShape::Trapezoid),
            "roundrect" => Some(PadShape::RoundRect),
            "custom" => Some(PadShape::Custom),
            _ => Some(PadShape::Rect),
        })
        .unwrap_or(PadShape::Rect);

    let at = sexp
        .find("at")
        .map(parse_at_optional_rot)
        .unwrap_or_default();
    let size = sexp
        .find("size")
        .and_then(parse_point)
        .unwrap_or(Point::new(1.0, 1.0));
    let drill = parse_drill(sexp.find("drill"));
    let layers = parse_layers(sexp.find("layers"));
    let roundrect_rratio = sexp
        .find("roundrect_rratio")
        .and_then(|r| r.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .and_then(|s| s.parse().ok());

    Ok(Pad {
        name,
        ty,
        shape,
        at,
        size,
        drill,
        layers,
        roundrect_rratio,
    })
}

fn parse_drill(sexp: Option<&Sexp>) -> Option<Drill> {
    let s = sexp?;
    let rest = s.list_rest()?;
    let mut oval = false;
    let mut i = 0;
    if rest.first().and_then(|a| a.string()) == Some("oval") {
        oval = true;
        i = 1;
    }
    let w = rest.get(i).and_then(|a| a.string()).and_then(|s| s.parse().ok())?;
    let h = rest.get(i + 1).and_then(|a| a.string()).and_then(|s| s.parse().ok()).unwrap_or(w);
    Some(Drill { oval, width: w, height: h })
}

fn parse_layers(sexp: Option<&Sexp>) -> Vec<String> {
    let Some(s) = sexp else { return vec![] };
    let Some(rest) = s.list_rest() else { return vec![] };
    rest.iter()
        .filter_map(|a| a.string().map(String::from))
        .collect()
}

fn parse_fp_line(sexp: &Sexp) -> Result<FpLine> {
    let start = sexp
        .find("start")
        .and_then(parse_point)
        .ok_or_else(|| Error::Parse {
            message: "fp_line: missing start".into(),
            offset: None,
        })?;
    let end = sexp
        .find("end")
        .and_then(parse_point)
        .ok_or_else(|| Error::Parse {
            message: "fp_line: missing end".into(),
            offset: None,
        })?;
    let width = extract_width_from_stroke(sexp);
    let width = if width == 0.0 {
        sexp.find("width")
            .and_then(|w| w.list_rest())
            .and_then(|r| r.first())
            .and_then(|a| a.string())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.12)
    } else {
        width
    };
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.SilkS")
        .to_string();

    Ok(FpLine { start, end, layer, width })
}

fn parse_fp_rect(sexp: &Sexp) -> Result<FpRect> {
    let start = sexp.find("start").and_then(parse_point).unwrap_or_default();
    let end = sexp.find("end").and_then(parse_point).unwrap_or_default();
    let width = extract_width_from_stroke(sexp);
    let width = if width == 0.0 {
        sexp.find("width")
            .and_then(|w| w.list_rest())
            .and_then(|r| r.first())
            .and_then(|a| a.string())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.12)
    } else {
        width
    };
    let fill = sexp
        .find("fill")
        .and_then(|f| f.list_rest()?.first())
        .and_then(|a| a.string())
        == Some("yes");
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.SilkS")
        .to_string();

    Ok(FpRect {
        start,
        end,
        layer,
        width,
        fill,
    })
}

fn parse_fp_circle(sexp: &Sexp) -> Result<FpCircle> {
    let center = sexp.find("center").and_then(parse_point).unwrap_or_default();
    let end = sexp.find("end").and_then(parse_point).unwrap_or_default();
    let width = extract_width_from_stroke(sexp);
    let width = if width == 0.0 {
        sexp.find("width")
            .and_then(|w| w.list_rest())
            .and_then(|r| r.first())
            .and_then(|a| a.string())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.12)
    } else {
        width
    };
    let fill = sexp
        .find("fill")
        .and_then(|f| f.list_rest()?.first())
        .and_then(|a| a.string())
        == Some("yes");
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.SilkS")
        .to_string();

    Ok(FpCircle {
        center,
        end,
        layer,
        width,
        fill,
    })
}

fn parse_fp_arc(sexp: &Sexp) -> Result<FpArc> {
    let start = sexp
        .find("start")
        .and_then(parse_point)
        .ok_or_else(|| Error::Parse {
            message: "fp_arc: missing start".into(),
            offset: None,
        })?;
    let mid = sexp
        .find("mid")
        .and_then(parse_point)
        .ok_or_else(|| Error::Parse {
            message: "fp_arc: missing mid".into(),
            offset: None,
        })?;
    let end = sexp
        .find("end")
        .and_then(parse_point)
        .ok_or_else(|| Error::Parse {
            message: "fp_arc: missing end".into(),
            offset: None,
        })?;
    let width = extract_width_from_stroke(sexp);
    let width = if width == 0.0 {
        sexp.find("width")
            .and_then(|w| w.list_rest())
            .and_then(|r| r.first())
            .and_then(|a| a.string())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.12)
    } else {
        width
    };
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.SilkS")
        .to_string();

    Ok(FpArc {
        start,
        mid,
        end,
        layer,
        width,
    })
}

fn parse_fp_poly(sexp: &Sexp) -> Result<FpPoly> {
    let pts_sexp = sexp
        .find("pts")
        .ok_or_else(|| Error::Parse {
            message: "fp_poly: missing pts".into(),
            offset: None,
        })?;
    let pts = pts_sexp
        .list_rest()
        .unwrap_or(&[])
        .iter()
        .filter_map(parse_point)
        .collect();
    let width = extract_width_from_stroke(sexp);
    let width = if width == 0.0 {
        sexp.find("width")
            .and_then(|w| w.list_rest())
            .and_then(|r| r.first())
            .and_then(|a| a.string())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.12)
    } else {
        width
    };
    let fill = sexp
        .find("fill")
        .and_then(|f| f.list_rest()?.first())
        .and_then(|a| a.string())
        == Some("yes");
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.SilkS")
        .to_string();

    Ok(FpPoly {
        pts,
        layer,
        width,
        fill,
    })
}

fn parse_fp_curve(sexp: &Sexp) -> Result<FpCurve> {
    let pts_sexp = sexp.find("pts").ok_or_else(|| Error::Parse {
        message: "fp_curve: missing pts".into(),
        offset: None,
    })?;
    let pts = pts_sexp
        .list_rest()
        .unwrap_or(&[])
        .iter()
        .filter_map(parse_point)
        .collect();
    let width = extract_width_from_stroke(sexp);
    let width = if width == 0.0 {
        sexp.find("width")
            .and_then(|w| w.list_rest())
            .and_then(|r| r.first())
            .and_then(|a| a.string())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.12)
    } else {
        width
    };
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.SilkS")
        .to_string();

    Ok(FpCurve { pts, layer, width })
}

fn parse_fp_text(sexp: &Sexp) -> Result<FpText> {
    let rest = sexp.list_rest().unwrap_or(&[]);
    let ty = rest.get(0).and_then(|a| a.string()).unwrap_or("user").to_string();
    let text = rest.get(1).and_then(|a| a.string()).unwrap_or("").to_string();
    let at = sexp.find("at").map(parse_at_optional_rot).unwrap_or_default();
    let layer = sexp
        .find("layer")
        .and_then(|l| l.list_rest())
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .unwrap_or("F.Fab")
        .to_string();

    Ok(FpText { ty, text, at, layer })
}

fn parse_model(sexp: &Sexp) -> Result<Model> {
    let path = sexp
        .list_rest()
        .and_then(|r| r.first())
        .and_then(|a| a.string())
        .ok_or_else(|| Error::Parse {
            message: "model: missing path".into(),
            offset: None,
        })?
        .to_string();
    let offset = sexp
        .find("at")
        .and_then(|a| a.find("xyz"))
        .and_then(|x| {
            let r = x.list_rest()?;
            let x = r.get(0)?.string()?.parse().ok()?;
            let y = r.get(1)?.string()?.parse().ok()?;
            let z = r.get(2)?.string()?.parse().ok()?;
            Some((x, y, z))
        })
        .unwrap_or((0.0, 0.0, 0.0));
    let scale = sexp
        .find("scale")
        .and_then(|s| s.find("xyz"))
        .and_then(|x| {
            let r = x.list_rest()?;
            let x = r.get(0)?.string()?.parse().ok()?;
            let y = r.get(1)?.string()?.parse().ok()?;
            let z = r.get(2)?.string()?.parse().ok()?;
            Some((x, y, z))
        })
        .unwrap_or((1.0, 1.0, 1.0));
    let rotate = sexp
        .find("rotate")
        .and_then(|r| r.find("xyz"))
        .and_then(|x| {
            let r = x.list_rest()?;
            let x = r.get(0)?.string()?.parse().ok()?;
            let y = r.get(1)?.string()?.parse().ok()?;
            let z = r.get(2)?.string()?.parse().ok()?;
            Some((x, y, z))
        })
        .unwrap_or((0.0, 0.0, 0.0));

    Ok(Model {
        path,
        offset,
        scale,
        rotate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_footprint() {
        let s = r#"(footprint "Test"
            (version 20241229)
            (layer "F.Cu")
            (descr "Test footprint")
            (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu" "*.Mask"))
        )"#;
        let fp = parse_footprint(s).expect("parse");
        assert_eq!(fp.name, "Test");
        assert_eq!(fp.layer, "F.Cu");
        assert_eq!(fp.pads().count(), 1);
        let pad = fp.pads().next().unwrap();
        assert_eq!(pad.name, "1");
        assert_eq!(pad.shape, PadShape::Circle);
    }

    #[test]
    fn parse_kicad9_with_stroke_and_fp_arc() {
        let s = r#"(footprint "LED_D3.0mm"
            (version 20241229)
            (layer "F.Cu")Dra
            (fp_line (start 0 0) (end 1 0) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
            (fp_arc (start -0.29 -1.23) (mid 1.36 -1.98) (end 2.94 -1.08) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
            (pad "1" thru_hole circle (at -1.27 0) (size 1.8 1.8) (drill 1) (layers "*.Cu" "*.Mask"))
        )"#;
        let fp = parse_footprint(s).expect("parse");
        assert_eq!(fp.name, "LED_D3.0mm");
        let arcs: Vec<_> = fp.graphics().filter(|e| matches!(e, Element::FpArc(_))).collect();
        assert_eq!(arcs.len(), 1);
        assert_eq!(fp.pads().count(), 1);
    }

    #[test]
    fn parse_dip16_footprint_format() {
        // KiCad 9 DIP-16 format: property (effects), fp_rect, fp_text user, pad roundrect
        let s = r#"(footprint "DIP-16_W7.62mm"
            (version 20241229)
            (generator "kicad-footprint-generator")
            (layer "F.Cu")
            (descr "16-lead though-hole mounted DIP package")
            (tags "THT DIP DIL PDIP 2.54mm 7.62mm 300mil")
            (property "Reference" "REF**" (at 3.81 -2.33 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
            (property "Value" "DIP-16_W7.62mm" (at 3.81 20.11 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
            (attr through_hole)
            (fp_line (start 1.16 -1.33) (end 1.16 19.11) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
            (fp_arc (start 4.81 -1.33) (mid 3.81 -0.33) (end 2.81 -1.33) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
            (fp_rect (start -1.06 -1.52) (end 8.67 19.3) (stroke (width 0.05) (type solid)) (fill no) (layer "F.CrtYd"))
            (fp_text user "${REFERENCE}" (at 3.81 8.89 90) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
            (pad "1" thru_hole roundrect (at 0 0) (size 1.6 1.6) (drill 0.8) (layers "*.Cu" "*.Mask") (roundrect_rratio 0.15625))
            (pad "2" thru_hole circle (at 0 2.54) (size 1.6 1.6) (drill 0.8) (layers "*.Cu" "*.Mask"))
        )"#;
        let fp = parse_footprint(s).expect("parse");
        assert_eq!(fp.name, "DIP-16_W7.62mm");
        assert_eq!(fp.version.as_deref(), Some("20241229"));
        assert_eq!(fp.generator.as_deref(), Some("kicad-footprint-generator"));
        assert_eq!(fp.attr.as_deref(), Some("through_hole"));
        assert_eq!(fp.pads().count(), 2);
        let pads: Vec<_> = fp.pads().collect();
        assert_eq!(pads[0].name, "1");
        assert_eq!(pads[0].shape, PadShape::RoundRect);
        assert_eq!(pads[0].at.x, 0.0);
        assert_eq!(pads[0].at.y, 0.0);
        assert_eq!(pads[0].roundrect_rratio, Some(0.15625));
        assert_eq!(pads[1].name, "2");
        assert_eq!(pads[1].shape, PadShape::Circle);
        assert_eq!(pads[1].at.y, 2.54);
        let lines: Vec<_> = fp.graphics().filter(|e| matches!(e, Element::FpLine(_))).collect();
        assert_eq!(lines.len(), 1);
        let arcs: Vec<_> = fp.graphics().filter(|e| matches!(e, Element::FpArc(_))).collect();
        assert_eq!(arcs.len(), 1);
        let rects: Vec<_> = fp.graphics().filter(|e| matches!(e, Element::FpRect(_))).collect();
        assert_eq!(rects.len(), 1);
        let texts: Vec<_> = fp.graphics().filter(|e| matches!(e, Element::FpText(_))).collect();
        assert_eq!(texts.len(), 1);
    }
}
