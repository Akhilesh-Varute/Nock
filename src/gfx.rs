//! Rendering: Direct2D (software, GDI-compatible) + DirectWrite drawing into a
//! 32-bit premultiplied DIB that is pushed to a layered window with
//! `UpdateLayeredWindow`. Nothing here runs unless something changes on screen.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_numerics::Matrix3x2;

// ---------------------------------------------------------------- basic types

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.x + self.w,
            bottom: self.y + self.h,
        }
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w * 0.5
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h * 0.5
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }
    pub const fn white(a: f32) -> Self {
        Color(1.0, 1.0, 1.0, a)
    }
    /// Multiply alpha (used for fading content in and out).
    pub fn fade(self, k: f32) -> Self {
        Color(self.0, self.1, self.2, self.3 * k.clamp(0.0, 1.0))
    }
    pub fn d2d(&self) -> D2D1_COLOR_F {
        D2D1_COLOR_F {
            r: self.0,
            g: self.1,
            b: self.2,
            a: self.3,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct Fonts {
    pub title: IDWriteTextFormat,
    pub body: IDWriteTextFormat,
    pub small: IDWriteTextFormat,
    pub menu: IDWriteTextFormat,
    pub tiny: IDWriteTextFormat,
    pub big: IDWriteTextFormat,
    pub icon14: IDWriteTextFormat,
    pub icon18: IDWriteTextFormat,
    pub icon22: IDWriteTextFormat,
}

// ----------------------------------------------------------------------- Gfx

pub struct Gfx {
    pub wic: IWICImagingFactory,
    rt: ID2D1DCRenderTarget,
    brush: ID2D1SolidColorBrush,
    memdc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    pub fonts: Fonts,
}

impl Gfx {
    /// `max_w` / `max_h` are the largest window size in *physical* pixels.
    pub fn new(max_w: i32, max_h: i32, scale: f32) -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;

            // Memory DC + top-down 32bpp DIB.
            let memdc = CreateCompatibleDC(None);
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = max_w;
            bmi.bmiHeader.biHeight = -max_h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = CreateDIBSection(Some(memdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(memdc, dib.into());

            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0 * scale,
                dpiY: 96.0 * scale,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = d2d.CreateDCRenderTarget(&props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&Color::white(1.0).d2d(), None)?;

            let fonts = make_fonts(&dw)?;
            Ok(Gfx {
                wic,
                rt,
                brush,
                memdc,
                dib,
                old,
                fonts,
            })
        }
    }

    // ------------------------------------------------------------ frame ----

    pub fn begin(&self, pw: i32, ph: i32) -> Result<()> {
        unsafe {
            let r = RECT {
                left: 0,
                top: 0,
                right: pw,
                bottom: ph,
            };
            self.rt.BindDC(self.memdc, &r)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
        }
        Ok(())
    }

    pub fn end(&self) -> Result<()> {
        unsafe { self.rt.EndDraw(None, None) }
    }

    pub fn present(&self, hwnd: HWND, x: i32, y: i32, pw: i32, ph: i32) -> Result<()> {
        unsafe {
            let dst = POINT { x, y };
            let size = SIZE { cx: pw, cy: ph };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&dst),
                Some(&size),
                Some(self.memdc),
                Some(&src),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
        }
    }

    // -------------------------------------------------------- primitives ----

    pub fn fill_rrect(&self, r: Rect, rad: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let rr = D2D1_ROUNDED_RECT {
                rect: r.d2d(),
                radiusX: rad,
                radiusY: rad,
            };
            self.rt.FillRoundedRectangle(&rr, &self.brush);
        }
    }

    pub fn stroke_rrect(&self, r: Rect, rad: f32, c: Color, width: f32) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let rr = D2D1_ROUNDED_RECT {
                rect: r.d2d(),
                radiusX: rad,
                radiusY: rad,
            };
            self.rt.DrawRoundedRectangle(&rr, &self.brush, width, None);
        }
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, rad: f32, c: Color) {
        unsafe {
            self.brush.SetColor(&c.d2d());
            let e = D2D1_ELLIPSE {
                point: windows_numerics::Vector2 { X: cx, Y: cy },
                radiusX: rad,
                radiusY: rad,
            };
            self.rt.FillEllipse(&e, &self.brush);
        }
    }

    pub fn text(&self, s: &str, r: Rect, f: &IDWriteTextFormat, c: Color, align: Align) {
        if s.is_empty() || c.3 <= 0.003 {
            return;
        }
        unsafe {
            let w: Vec<u16> = s.encode_utf16().collect();
            let _ = f.SetTextAlignment(match align {
                Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            self.brush.SetColor(&c.d2d());
            self.rt.DrawText(
                &w,
                f,
                &r.d2d(),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Draw a glyph from the icon font, centred in `r`.
    pub fn icon(&self, glyph: char, r: Rect, f: &IDWriteTextFormat, c: Color) {
        let mut buf = [0u8; 4];
        let s = glyph.encode_utf8(&mut buf);
        self.text(s, r, f, c, Align::Center);
    }

    /// Fill a rounded rect with a bitmap (used for album art / app icons).
    pub fn draw_bitmap(&self, bmp: &ID2D1Bitmap, r: Rect, rad: f32, alpha: f32) {
        unsafe {
            let size = bmp.GetSize();
            if size.width <= 0.0 || size.height <= 0.0 {
                return;
            }
            let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
            };
            if let Ok(bb) = self.rt.CreateBitmapBrush(bmp, Some(&props), None) {
                bb.SetTransform(&Matrix3x2 {
                    M11: r.w / size.width,
                    M12: 0.0,
                    M21: 0.0,
                    M22: r.h / size.height,
                    M31: r.x,
                    M32: r.y,
                });
                bb.SetOpacity(alpha.clamp(0.0, 1.0));
                let rr = D2D1_ROUNDED_RECT {
                    rect: r.d2d(),
                    radiusX: rad,
                    radiusY: rad,
                };
                self.rt.FillRoundedRectangle(&rr, &bb);
            }
        }
    }

    // ------------------------------------------------------------ images ----

    /// Decode encoded image bytes (jpeg/png/...) into a GPU-less D2D bitmap and
    /// also return the average colour (cheap 8x8 downscale) for accent tinting.
    pub fn decode_image(&self, bytes: &[u8]) -> Result<(ID2D1Bitmap, Color)> {
        unsafe {
            let stream = SHCreateMemStream(Some(bytes)).ok_or_else(|| Error::from(E_FAIL))?;
            let dec = self.wic.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )?;
            let frame = dec.GetFrame(0)?;
            let conv = self.wic.CreateFormatConverter()?;
            conv.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            let bmp = self.rt.CreateBitmapFromWicBitmap(&conv, None)?;

            let sc = self.wic.CreateBitmapScaler()?;
            sc.Initialize(&conv, 8, 8, WICBitmapInterpolationModeFant)?;
            let mut px = [0u8; 8 * 8 * 4];
            sc.CopyPixels(std::ptr::null(), 8 * 4, &mut px)?;
            let (mut r, mut g, mut b, mut n) = (0f32, 0f32, 0f32, 0f32);
            for p in px.chunks_exact(4) {
                if p[3] > 8 {
                    let a = p[3] as f32 / 255.0;
                    // premultiplied -> straight
                    b += p[0] as f32 / 255.0 / a;
                    g += p[1] as f32 / 255.0 / a;
                    r += p[2] as f32 / 255.0 / a;
                    n += 1.0;
                }
            }
            let col = if n > 0.0 {
                Color(r / n, g / n, b / n, 1.0)
            } else {
                Color(0.4, 0.6, 1.0, 1.0)
            };
            Ok((bmp, col))
        }
    }

    /// Icon of a file (exe, folder, ...) as a D2D bitmap.
    pub fn bitmap_from_hicon(&self, icon: HICON) -> Result<ID2D1Bitmap> {
        unsafe {
            let wb = self.wic.CreateBitmapFromHICON(icon)?;
            let conv = self.wic.CreateFormatConverter()?;
            conv.Initialize(
                &wb,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )?;
            self.rt.CreateBitmapFromWicBitmap(&conv, None)
        }
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.memdc, self.old);
            let _ = DeleteObject(self.dib.into());
            let _ = DeleteDC(self.memdc);
        }
    }
}

// -------------------------------------------------------------------- fonts

fn make_fonts(dw: &IDWriteFactory) -> Result<Fonts> {
    unsafe {
        let icon_family = if font_exists(dw, w!("Segoe Fluent Icons")) {
            w!("Segoe Fluent Icons")
        } else {
            w!("Segoe MDL2 Assets")
        };
        let text =
            |family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32| -> Result<IDWriteTextFormat> {
                let f = dw.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                let trim = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                if let Ok(sign) = dw.CreateEllipsisTrimmingSign(&f) {
                    let _ = f.SetTrimming(&trim, &sign);
                }
                Ok(f)
            };
        let ui = w!("Segoe UI");
        Ok(Fonts {
            title: text(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 17.0)?,
            body: text(ui, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
            small: text(ui, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
            menu: text(ui, DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.0)?,
            tiny: text(ui, DWRITE_FONT_WEIGHT_NORMAL, 11.0)?,
            big: text(ui, DWRITE_FONT_WEIGHT_LIGHT, 34.0)?,
            icon14: text(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
            icon18: text(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 18.0)?,
            icon22: text(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 22.0)?,
        })
    }
}

fn font_exists(dw: &IDWriteFactory, family: PCWSTR) -> bool {
    unsafe {
        let mut coll: Option<IDWriteFontCollection> = None;
        if dw.GetSystemFontCollection(&mut coll, false).is_err() {
            return false;
        }
        let Some(coll) = coll else { return false };
        let mut index = 0u32;
        let mut exists = BOOL(0);
        if coll
            .FindFamilyName(family, &mut index, &mut exists)
            .is_err()
        {
            return false;
        }
        exists.as_bool()
    }
}
