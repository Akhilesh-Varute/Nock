//! Linux entry point (work in progress): a borderless winit window that draws
//! the notch pill through the shared `gfx` API and shows it with softbuffer.

use std::num::NonZeroU32;
use std::rc::Rc;

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::x11::{EventLoopBuilderExtX11, WindowAttributesExtX11, WindowType};
use winit::window::{Window, WindowId, WindowLevel};

use crate::gfx::*;

const W: f32 = 220.0;
const H: f32 = 34.0;

#[derive(Default)]
struct App {
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    gfx: Option<Gfx>,
}

impl App {
    fn draw(&mut self) {
        let (Some(win), Some(surface)) = (&self.window, &mut self.surface) else { return };
        let scale = win.scale_factor() as f32;
        let (pw, ph) = ((W * scale).ceil() as u32, (H * scale).ceil() as u32);
        if self.gfx.is_none() {
            self.gfx = Gfx::new(pw as i32, ph as i32, scale).ok();
        }
        let Some(g) = &self.gfx else { return };
        let _ = g.begin(pw as i32, ph as i32);
        g.fill_rrect(Rect::new(0.0, -20.0, W, H + 20.0), 17.0, Color::rgb(0, 0, 0));
        g.text("Nock", Rect::new(0.0, 0.0, W, H), &g.fonts.menu, Color::white(0.9), Align::Center);
        let _ = g.end();

        let (gw, gh, rgba) = g.pixels();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(gw.min(pw)), NonZeroU32::new(gh.min(ph))) else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        for y in 0..nh.get() {
            for x in 0..nw.get() {
                let s = ((y * gw + x) * 4) as usize;
                let (r, gr, b, a) = (rgba[s] as u32, rgba[s + 1] as u32, rgba[s + 2] as u32, rgba[s + 3] as u32);
                buf[(y * nw.get() + x) as usize] = a << 24 | r << 16 | gr << 8 | b;
            }
        }
        let _ = buf.present();
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("Nock")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_x11_window_type(vec![WindowType::Dock])
            .with_inner_size(LogicalSize::new(W as f64, H as f64));
        let Ok(win) = el.create_window(attrs) else { return };
        // top-centre of the primary monitor (X11 only: Wayland ignores client positions)
        if let Some(m) = el.primary_monitor().or_else(|| win.current_monitor()) {
            let (mp, ms) = (m.position(), m.size());
            let x = mp.x + (ms.width as i32 - win.outer_size().width as i32) / 2;
            win.set_outer_position(PhysicalPosition::new(x, mp.y));
        }
        let win = Rc::new(win);
        if let Ok(ctx) = softbuffer::Context::new(win.clone()) {
            self.surface = softbuffer::Surface::new(&ctx, win.clone()).ok();
        }
        self.window = Some(win);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::RedrawRequested => self.draw(),
            _ => {}
        }
    }
}

pub fn run() {
    // ponytail: X11 only (Xwayland on Wayland desktops); native Wayland needs layer-shell
    let el = EventLoop::builder().with_x11().build().expect("event loop");
    if let Err(e) = el.run_app(&mut App::default()) {
        eprintln!("nock: {e}");
    }
}
