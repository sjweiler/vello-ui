/// A tightly packed premultiplied RGBA8 upload frame.
///
/// This is retained as a low-level interop type for clients that already own
/// pixels; VUI's desktop host does not rasterize paint lists on the CPU.
pub struct RasterFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}
