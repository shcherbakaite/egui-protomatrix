//! Data types for KiCad 9 footprints.


/// A KiCad 9 footprint.
#[derive(Debug, Clone)]
pub struct Footprint {
    pub name: String,
    pub version: Option<String>,
    pub generator: Option<String>,
    pub layer: String,
    pub description: Option<String>,
    pub tags: Option<String>,
    pub attr: Option<String>, // "smd" | "through_hole"
    pub properties: Vec<Property>,
    pub elements: Vec<Element>,
    pub model: Option<Model>,
}

/// Property (Reference, Value, Datasheet, Description).
#[derive(Debug, Clone)]
pub struct Property {
    pub key: String,
    pub value: String,
}

/// Graphic or pad element.
#[derive(Debug, Clone)]
pub enum Element {
    Pad(Pad),
    FpLine(FpLine),
    FpRect(FpRect),
    FpCircle(FpCircle),
    FpArc(FpArc),
    FpPoly(FpPoly),
    FpCurve(FpCurve),
    FpText(FpText),
}

/// Pad definition.
#[derive(Debug, Clone)]
pub struct Pad {
    pub name: String,
    pub ty: PadType,
    pub shape: PadShape,
    pub at: Point,
    pub size: Point,
    pub drill: Option<Drill>,
    pub layers: Vec<String>,
    pub roundrect_rratio: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadType {
    Smd,
    ThruHole,
    NpThruHole,
    Connect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadShape {
    Circle,
    Rect,
    Oval,
    Trapezoid,
    RoundRect,
    Custom,
}

#[derive(Debug, Clone)]
pub struct Drill {
    pub oval: bool,
    pub width: f64,
    pub height: f64,
}

/// Line segment.
#[derive(Debug, Clone)]
pub struct FpLine {
    pub start: Point,
    pub end: Point,
    pub layer: String,
    pub width: f64,
}

/// Rectangle.
#[derive(Debug, Clone)]
pub struct FpRect {
    pub start: Point,
    pub end: Point,
    pub layer: String,
    pub width: f64,
    pub fill: bool,
}

/// Circle.
#[derive(Debug, Clone)]
pub struct FpCircle {
    pub center: Point,
    pub end: Point,  // point on radius
    pub layer: String,
    pub width: f64,
    pub fill: bool,
}

/// Arc (KiCad 9: start, mid, end).
#[derive(Debug, Clone)]
pub struct FpArc {
    pub start: Point,
    pub mid: Point,
    pub end: Point,
    pub layer: String,
    pub width: f64,
}

/// Polygon.
#[derive(Debug, Clone)]
pub struct FpPoly {
    pub pts: Vec<Point>,
    pub layer: String,
    pub width: f64,
    pub fill: bool,
}

/// Cubic Bezier curve (4 points).
#[derive(Debug, Clone)]
pub struct FpCurve {
    pub pts: Vec<Point>,
    pub layer: String,
    pub width: f64,
}

/// Text.
#[derive(Debug, Clone)]
pub struct FpText {
    pub ty: String,  // "reference" | "value" | "user"
    pub text: String,
    pub at: Point,
    pub layer: String,
}

/// 2D point (mm).
#[derive(Debug, Clone, Copy, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// 3D model reference.
#[derive(Debug, Clone)]
pub struct Model {
    pub path: String,
    pub offset: (f64, f64, f64),
    pub scale: (f64, f64, f64),
    pub rotate: (f64, f64, f64),
}

impl Footprint {
    pub fn pads(&self) -> impl Iterator<Item = &Pad> {
        self.elements.iter().filter_map(|e| {
            if let Element::Pad(p) = e {
                Some(p)
            } else {
                None
            }
        })
    }

    pub fn graphics(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| !matches!(e, Element::Pad(_)))
    }
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}
