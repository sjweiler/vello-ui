#[cfg(target_os = "linux")]
mod linux {
    use std::time::{Duration, Instant};

    use vui_ash::{GpuSceneBuilder, VulkanPresenter};
    use vui_core::{Color, PaintList, PaintOp, Rect, Size, UiFrame};
    use winit::{
        application::ApplicationHandler,
        dpi::{LogicalSize, PhysicalSize},
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        window::{Window, WindowId},
    };

    const FRAME_LIMIT: u32 = 240;
    const TIME_LIMIT: Duration = Duration::from_secs(45);

    pub fn run() {
        if std::env::var_os("VUI_RUN_WINDOWED_SWAPCHAIN").is_none() {
            eprintln!(
                "windowed Vulkan swapchain stress test skipped: \
                 set VUI_RUN_WINDOWED_SWAPCHAIN=1 to enable"
            );
            return;
        }
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            eprintln!("windowed Vulkan swapchain stress test skipped: no display server");
            return;
        }

        let event_loop = EventLoop::new().expect("create windowed stress-test event loop");
        event_loop.set_control_flow(ControlFlow::Poll);
        let mut app = SwapchainStress::new();
        event_loop
            .run_app(&mut app)
            .expect("run windowed stress-test event loop");
        if let Some(error) = app.error {
            panic!("windowed Vulkan swapchain stress test failed: {error}");
        }
        assert_eq!(app.frames, FRAME_LIMIT);
    }

    struct SwapchainStress {
        // The presenter must be dropped before the window that owns its raw handles.
        presenter: Option<VulkanPresenter>,
        window: Option<Window>,
        scene_builder: GpuSceneBuilder,
        frames: u32,
        started: Instant,
        error: Option<String>,
    }

    impl SwapchainStress {
        fn new() -> Self {
            Self {
                presenter: None,
                window: None,
                scene_builder: GpuSceneBuilder::new(),
                frames: 0,
                started: Instant::now(),
                error: None,
            }
        }

        fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl Into<String>) {
            self.error = Some(error.into());
            event_loop.exit();
        }

        fn redraw(&mut self, event_loop: &ActiveEventLoop) {
            let Some(window) = self.window.as_ref() else {
                return;
            };
            let size = window.inner_size();
            if size.width == 0 || size.height == 0 {
                window.request_redraw();
                return;
            }
            let frame = stress_frame(size, self.frames);
            let scene = match self.scene_builder.build(&frame, 1.0) {
                Ok(scene) => scene,
                Err(error) => {
                    self.fail(event_loop, format!("build frame {}: {error}", self.frames));
                    return;
                }
            };
            if let Err(error) = self
                .presenter
                .as_mut()
                .expect("presenter exists after resume")
                .present_gpu(&scene)
            {
                self.fail(
                    event_loop,
                    format!("present frame {}: {error}", self.frames),
                );
                return;
            }

            self.frames += 1;
            if self.frames == FRAME_LIMIT {
                event_loop.exit();
                return;
            }
            if self.frames.is_multiple_of(12) {
                let extent = if (self.frames / 12).is_multiple_of(2) {
                    PhysicalSize::new(640, 480)
                } else {
                    PhysicalSize::new(800, 600)
                };
                let _ = window.request_inner_size(extent);
            }
            window.request_redraw();
        }
    }

    impl ApplicationHandler for SwapchainStress {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return;
            }
            let attributes = Window::default_attributes()
                .with_title("VUI Vulkan swapchain stress test")
                .with_inner_size(LogicalSize::new(640.0, 480.0));
            let window = match event_loop.create_window(attributes) {
                Ok(window) => window,
                Err(error) => {
                    self.fail(event_loop, format!("create window: {error}"));
                    return;
                }
            };
            let size = window.inner_size();
            let presenter =
                match VulkanPresenter::new(&window, size.width.max(1), size.height.max(1)) {
                    Ok(presenter) => presenter,
                    Err(error) => {
                        self.fail(event_loop, format!("create Vulkan presenter: {error}"));
                        return;
                    }
                };
            self.presenter = Some(presenter);
            self.window = Some(window);
            self.window
                .as_ref()
                .expect("window was stored")
                .request_redraw();
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _window_id: WindowId,
            event: WindowEvent,
        ) {
            match event {
                WindowEvent::RedrawRequested => self.redraw(event_loop),
                WindowEvent::CloseRequested => {
                    self.fail(event_loop, "window closed before stress test completed");
                }
                WindowEvent::Resized(_) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                _ => {}
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if self.started.elapsed() > TIME_LIMIT {
                self.fail(
                    event_loop,
                    format!("timed out after {} frames", self.frames),
                );
            } else if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn stress_frame(size: PhysicalSize<u32>, iteration: u32) -> UiFrame {
        let mut paint = PaintList::default();
        paint.push(PaintOp::FillRect {
            rect: Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
            color: Color::rgb(18, 22, 30),
            radius: 0.0,
        });

        let descending = iteration % 2 == 1;
        for index in 0_u32..64 {
            let sorted = if descending { 63 - index } else { index };
            let column = index % 8;
            let row = index / 8;
            paint.push(PaintOp::FillRect {
                rect: Rect::new(
                    16.0 + f64::from(column) * 54.0,
                    16.0 + f64::from(row) * 42.0,
                    48.0,
                    36.0,
                ),
                color: Color::rgb(
                    u8::try_from((sorted * 37) % 255).expect("color channel is in range"),
                    u8::try_from((sorted * 67) % 255).expect("color channel is in range"),
                    u8::try_from((sorted * 97) % 255).expect("color channel is in range"),
                ),
                radius: 5.0,
            });
        }

        UiFrame {
            viewport: Size::new(f64::from(size.width), f64::from(size.height)),
            damage: Vec::new(),
            paint,
            semantics: Vec::new(),
        }
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    linux::run();
}
