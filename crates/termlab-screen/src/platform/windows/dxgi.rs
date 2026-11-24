//! The picture through DXGI Desktop Duplication: the GPU hands over the desktop image when
//! (and only when) it changed, which is faster and lighter than GDI's copy. The pointer
//! isn't in it; when the browser doesn't draw the pointer itself, it's drawn in here.
//!
//! Falls back to GDI (the caller's job) on rotated displays, HDR desktops, or when the
//! duplication can't start.

use anyhow::{Context, bail};
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_DESC,
    DXGI_OUTDUPL_FRAME_INFO, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource,
};
use windows::core::Interface;

use super::super::cursor;
use super::super::soft::{Grab, Image, Pointer};
use super::cursor::WinPointer;

pub struct Dxgi {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    output: IDXGIOutput1,
    dup: Option<IDXGIOutputDuplication>,
    staging: ID3D11Texture2D,
    rect: RECT,
    w: u32,
    h: u32,
    /// The last desktop image (no pointer), tightly packed BGRA.
    last: Option<Vec<u8>>,
    /// Draws the pointer in, when the browser doesn't.
    pointer: Option<WinPointer>,
}

impl Dxgi {
    /// Duplicates the output showing `rect` (desktop coordinates, physical pixels).
    pub fn new(rect: RECT, scale: f64, draw_cursor: bool) -> anyhow::Result<Self> {
        let (w, h) = (
            (rect.right - rect.left) as u32,
            (rect.bottom - rect.top) as u32,
        );
        // SAFETY: DXGI/D3D11 set-up on valid objects; out pointers are filled on success.
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let mut found = None;
            'adapters: for a in 0.. {
                let Ok(adapter) = factory.EnumAdapters1(a) else {
                    break;
                };
                for o in 0.. {
                    let Ok(output) = adapter.EnumOutputs(o) else {
                        break;
                    };
                    let d = output.GetDesc()?;
                    if d.DesktopCoordinates == rect {
                        if d.Rotation != DXGI_MODE_ROTATION_IDENTITY
                            && d.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED
                        {
                            bail!("a rotated display");
                        }
                        found = Some((adapter.clone(), output.cast::<IDXGIOutput1>()?));
                        break 'adapters;
                    }
                }
            }
            let (adapter, output): (IDXGIAdapter1, IDXGIOutput1) =
                found.context("no DXGI output for this display")?;
            let mut device = None;
            let mut ctx = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )?;
            let (device, ctx) = (
                device.context("no D3D11 device")?,
                ctx.context("no D3D11 context")?,
            );
            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            let mut me = Dxgi {
                device,
                ctx,
                output,
                dup: None,
                staging: staging.context("no staging texture")?,
                rect,
                w,
                h,
                last: None,
                pointer: draw_cursor.then(|| WinPointer::new(scale)),
            };
            // Must work now (else GDI is used): fails on HDR desktops, for one.
            me.duplicate()?;
            Ok(me)
        }
    }

    fn duplicate(&mut self) -> anyhow::Result<()> {
        // SAFETY: a valid output and device.
        let dup = unsafe { self.output.DuplicateOutput(&self.device)? };
        // SAFETY: a valid duplication.
        let d: DXGI_OUTDUPL_DESC = unsafe { dup.GetDesc() };
        if d.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
            || d.ModeDesc.Width != self.w
            || d.ModeDesc.Height != self.h
        {
            bail!("the desktop's format or size changed");
        }
        self.dup = Some(dup);
        Ok(())
    }

    /// Copies a new desktop image into `last`. False if there was none.
    fn next(&mut self) -> anyhow::Result<bool> {
        if self.dup.is_none() && self.duplicate().is_err() {
            // The secure desktop, a mode change: try again later.
            return Ok(false);
        }
        let Some(dup) = &self.dup else {
            return Ok(false);
        };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        // SAFETY: valid duplication; a frame got is released below.
        match unsafe { dup.AcquireNextFrame(0, &mut info, &mut res) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                self.dup = None;
                return Ok(false);
            }
            Err(e) => return Err(e.into()),
        }
        let got = (|| -> anyhow::Result<bool> {
            // Only the pointer moved: no new image.
            if info.LastPresentTime == 0 {
                return Ok(false);
            }
            let tex: ID3D11Texture2D = res.as_ref().context("no frame")?.cast()?;
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            // SAFETY: copies on the GPU into our staging texture, then maps it for reading;
            // rows are `RowPitch` apart, each at least w*4 bytes.
            unsafe {
                self.ctx.CopyResource(&self.staging, &tex);
                self.ctx
                    .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
                let (w, h) = (self.w as usize, self.h as usize);
                let row = w * 4;
                let buf = self.last.get_or_insert_with(|| vec![0u8; row * h]);
                let src = map.pData as *const u8;
                for y in 0..h {
                    std::ptr::copy_nonoverlapping(
                        src.add(y * map.RowPitch as usize),
                        buf.as_mut_ptr().add(y * row),
                        row,
                    );
                }
                self.ctx.Unmap(&self.staging, 0);
            }
            Ok(true)
        })();
        // SAFETY: the frame acquired above.
        let _ = unsafe { dup.ReleaseFrame() };
        got
    }
}

impl Grab for Dxgi {
    fn grab(&mut self) -> anyhow::Result<Option<Image>> {
        let changed = self.next()?;
        let Some(last) = &self.last else {
            return Ok(None);
        };
        let (w, h) = (self.w, self.h);
        // With the pointer drawn in, every poll may differ (the pointer moves on its own);
        // the capture loop drops pictures that didn't change.
        let Some(p) = &mut self.pointer else {
            return Ok(changed.then(|| Image {
                data: last.clone(),
                width: w,
                height: h,
                stride: w as usize * 4,
            }));
        };
        let mut data = last.clone();
        if let Some(c) = p.read()
            && c.visible
            && let Some(shape) = &c.shape
        {
            cursor::blend(
                &mut data,
                w as usize,
                h as usize,
                shape,
                c.x as i32 - self.rect.left - shape.hot_x as i32,
                c.y as i32 - self.rect.top - shape.hot_y as i32,
            );
        }
        Ok(Some(Image {
            data,
            width: w,
            height: h,
            stride: w as usize * 4,
        }))
    }
}
