//! GAL descriptions to wgpu values: formats, usages, sampler, raster,
//! depth/stencil and blend state.

use crate::commands::{AttachmentLoadOp, AttachmentStoreOp, ClearColor};
use crate::error::{GalError, GalResult};
use crate::frame::PresentMode;
use crate::resources::{
    BlendMode, BufferUsage, CompareOp, CullMode, FrontFace, PrimitiveTopology, RasterYDirection,
    SamplerAddressMode, SamplerDesc, SamplerFilter, StencilFaceState, StencilOp, TextureFormat,
    TextureUsage,
};

/// The wgpu format a GAL format is stored in. `Rgb16Float` has no
/// three-channel wgpu equivalent and is stored as `Rgba16Float`; buffer
/// copies of it are rejected because the texel sizes differ.
pub(super) fn texture_format(format: TextureFormat) -> wgpu::TextureFormat {
    match format {
        TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        TextureFormat::Rgba16Float | TextureFormat::Rgb16Float => wgpu::TextureFormat::Rgba16Float,
        TextureFormat::Depth24Stencil8 => wgpu::TextureFormat::Depth24PlusStencil8,
        TextureFormat::Depth32Float => wgpu::TextureFormat::Depth32Float,
        TextureFormat::R8Uint => wgpu::TextureFormat::R8Uint,
        TextureFormat::R11fG11fB10f => wgpu::TextureFormat::Rg11b10Ufloat,
        TextureFormat::R32Float => wgpu::TextureFormat::R32Float,
        TextureFormat::R8Unorm => wgpu::TextureFormat::R8Unorm,
        TextureFormat::Rgba8Snorm => wgpu::TextureFormat::Rgba8Snorm,
        TextureFormat::R16Float => wgpu::TextureFormat::R16Float,
    }
}

/// The GAL format of a surface format, for formats a surface can offer.
pub(super) fn gal_surface_format(format: wgpu::TextureFormat) -> Option<TextureFormat> {
    match format {
        wgpu::TextureFormat::Rgba8Unorm => Some(TextureFormat::Rgba8Unorm),
        wgpu::TextureFormat::Bgra8Unorm => Some(TextureFormat::Bgra8Unorm),
        wgpu::TextureFormat::Rgba16Float => Some(TextureFormat::Rgba16Float),
        _ => None,
    }
}

/// Whether a float sample of `format` cannot be filtered, so a float
/// binding of it must be declared unfilterable.
pub(super) fn unfilterable_float(format: wgpu::TextureFormat, features: wgpu::Features) -> bool {
    match format {
        wgpu::TextureFormat::Depth32Float | wgpu::TextureFormat::Depth24PlusStencil8 => true,
        wgpu::TextureFormat::R32Float => !features.contains(wgpu::Features::FLOAT32_FILTERABLE),
        _ => false,
    }
}

pub(super) fn buffer_usages(usages: &[BufferUsage]) -> wgpu::BufferUsages {
    // Every buffer can be a copy endpoint and a storage binding: host
    // writes and unaligned copies are staged through copies and a byte-copy
    // compute pass.
    let mut native =
        wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE;
    for usage in usages {
        native |= match usage {
            BufferUsage::Vertex => wgpu::BufferUsages::VERTEX,
            BufferUsage::Index => wgpu::BufferUsages::INDEX,
            BufferUsage::Uniform => wgpu::BufferUsages::UNIFORM,
            BufferUsage::Indirect => wgpu::BufferUsages::INDIRECT,
            BufferUsage::Storage
            | BufferUsage::TransferSrc
            | BufferUsage::TransferDst
            | BufferUsage::HostRead
            | BufferUsage::HostWrite => wgpu::BufferUsages::empty(),
        };
    }
    native
}

pub(super) fn texture_usages(usages: &[TextureUsage]) -> wgpu::TextureUsages {
    let mut native = wgpu::TextureUsages::empty();
    for usage in usages {
        native |= match usage {
            TextureUsage::Sampled => wgpu::TextureUsages::TEXTURE_BINDING,
            TextureUsage::Storage => wgpu::TextureUsages::STORAGE_BINDING,
            TextureUsage::ColorAttachment | TextureUsage::DepthStencilAttachment => {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            }
            TextureUsage::TransferSrc | TextureUsage::HostRead => wgpu::TextureUsages::COPY_SRC,
            TextureUsage::TransferDst | TextureUsage::HostWrite => wgpu::TextureUsages::COPY_DST,
            TextureUsage::Present => wgpu::TextureUsages::COPY_SRC,
        };
    }
    native
}

fn filter(filter: SamplerFilter) -> wgpu::FilterMode {
    match filter {
        SamplerFilter::Nearest => wgpu::FilterMode::Nearest,
        SamplerFilter::Linear => wgpu::FilterMode::Linear,
    }
}

fn mipmap_filter(filter: SamplerFilter) -> wgpu::MipmapFilterMode {
    match filter {
        SamplerFilter::Nearest => wgpu::MipmapFilterMode::Nearest,
        SamplerFilter::Linear => wgpu::MipmapFilterMode::Linear,
    }
}

fn address(mode: SamplerAddressMode) -> wgpu::AddressMode {
    match mode {
        SamplerAddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        SamplerAddressMode::Repeat => wgpu::AddressMode::Repeat,
        SamplerAddressMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

pub(super) fn compare(op: CompareOp) -> wgpu::CompareFunction {
    match op {
        CompareOp::Always => wgpu::CompareFunction::Always,
        CompareOp::Less => wgpu::CompareFunction::Less,
        CompareOp::LessOrEqual => wgpu::CompareFunction::LessEqual,
        CompareOp::Equal => wgpu::CompareFunction::Equal,
        CompareOp::Greater => wgpu::CompareFunction::Greater,
    }
}

pub(super) fn sampler_descriptor(desc: &SamplerDesc) -> wgpu::SamplerDescriptor<'_> {
    wgpu::SamplerDescriptor {
        label: Some(&desc.label),
        address_mode_u: address(desc.address_u),
        address_mode_v: address(desc.address_v),
        address_mode_w: address(desc.address_w),
        mag_filter: filter(desc.mag_filter),
        min_filter: filter(desc.min_filter),
        mipmap_filter: mipmap_filter(desc.mip_filter),
        compare: desc.comparison.map(compare),
        ..Default::default()
    }
}

/// Triangle fans are drawn as triangle lists through generated indices.
pub(super) fn topology(topology: PrimitiveTopology) -> wgpu::PrimitiveTopology {
    match topology {
        PrimitiveTopology::Points => wgpu::PrimitiveTopology::PointList,
        PrimitiveTopology::Lines => wgpu::PrimitiveTopology::LineList,
        PrimitiveTopology::Triangles | PrimitiveTopology::TriangleFan => {
            wgpu::PrimitiveTopology::TriangleList
        }
    }
}

pub(super) fn cull_mode(mode: CullMode) -> Option<wgpu::Face> {
    match mode {
        CullMode::None => None,
        CullMode::Front => Some(wgpu::Face::Front),
        CullMode::Back => Some(wgpu::Face::Back),
    }
}

/// `FrontFace` keeps its clip-space meaning in both directions. A `Down`
/// pipeline's vertex shader is mirrored in Y, which mirrors winding, so the
/// declared face is swapped, exactly as the Vulkan backend does when it
/// drops its flipped viewport.
pub(super) fn front_face(face: FrontFace, direction: RasterYDirection) -> wgpu::FrontFace {
    match (face, direction) {
        (FrontFace::CounterClockwise, RasterYDirection::Up)
        | (FrontFace::Clockwise, RasterYDirection::Down) => wgpu::FrontFace::Ccw,
        (FrontFace::Clockwise, RasterYDirection::Up)
        | (FrontFace::CounterClockwise, RasterYDirection::Down) => wgpu::FrontFace::Cw,
    }
}

fn stencil_operation(op: StencilOp) -> wgpu::StencilOperation {
    match op {
        StencilOp::Keep => wgpu::StencilOperation::Keep,
        StencilOp::Replace => wgpu::StencilOperation::Replace,
    }
}

pub(super) fn stencil_face(face: &StencilFaceState) -> wgpu::StencilFaceState {
    wgpu::StencilFaceState {
        compare: compare(face.compare),
        fail_op: stencil_operation(face.fail_op),
        depth_fail_op: stencil_operation(face.depth_fail_op),
        pass_op: stencil_operation(face.pass_op),
    }
}

fn component(src_factor: wgpu::BlendFactor, dst_factor: wgpu::BlendFactor) -> wgpu::BlendComponent {
    wgpu::BlendComponent {
        src_factor,
        dst_factor,
        operation: wgpu::BlendOperation::Add,
    }
}

/// Blend state and write mask of one color attachment; the factors match
/// the Vulkan backend's table.
pub(super) fn color_blend(
    blend: BlendMode,
    attachment_index: usize,
) -> (Option<wgpu::BlendState>, wgpu::ColorWrites) {
    use wgpu::BlendFactor as F;
    let blend = match blend {
        BlendMode::AlphaFirstAttachmentOnly if attachment_index != 0 => BlendMode::Disabled,
        BlendMode::AlphaFirstAttachmentOnly => BlendMode::Alpha,
        other => other,
    };
    let state = |color, alpha| {
        (
            Some(wgpu::BlendState { color, alpha }),
            wgpu::ColorWrites::ALL,
        )
    };
    match blend {
        BlendMode::Disabled => (None, wgpu::ColorWrites::ALL),
        BlendMode::DepthMask => (None, wgpu::ColorWrites::empty()),
        BlendMode::Alpha => state(
            component(F::SrcAlpha, F::OneMinusSrcAlpha),
            component(F::One, F::OneMinusSrcAlpha),
        ),
        BlendMode::AlphaPreserveAlpha => state(
            component(F::SrcAlpha, F::OneMinusSrcAlpha),
            component(F::Zero, F::One),
        ),
        BlendMode::AlphaSource => state(
            component(F::SrcAlpha, F::OneMinusSrcAlpha),
            component(F::One, F::Zero),
        ),
        BlendMode::Premultiplied => state(
            component(F::One, F::OneMinusSrcAlpha),
            component(F::One, F::OneMinusSrcAlpha),
        ),
        BlendMode::Additive => state(component(F::One, F::One), component(F::One, F::One)),
        BlendMode::Invert => state(
            component(F::OneMinusDst, F::OneMinusSrc),
            component(F::One, F::Zero),
        ),
        BlendMode::Multiply => state(component(F::Dst, F::Zero), component(F::One, F::Zero)),
        BlendMode::Overlay => state(component(F::SrcAlpha, F::One), component(F::One, F::Zero)),
        BlendMode::SrcColorAdditive => state(component(F::Src, F::One), component(F::Zero, F::One)),
        BlendMode::InverseSrcColorModulate => state(
            component(F::Zero, F::OneMinusSrc),
            component(F::One, F::Zero),
        ),
        BlendMode::DoubleModulate => state(component(F::Dst, F::Src), component(F::One, F::Zero)),
        BlendMode::AlphaFirstAttachmentOnly => unreachable!("resolved above"),
    }
}

pub(super) fn color_load(
    op: AttachmentLoadOp,
    clear: Option<ClearColor>,
) -> wgpu::LoadOp<wgpu::Color> {
    match op {
        AttachmentLoadOp::Load => wgpu::LoadOp::Load,
        // Undefined contents may as well be a clear, which tilers prefer.
        AttachmentLoadOp::Clear | AttachmentLoadOp::DontCare => {
            let clear = clear.unwrap_or(ClearColor {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            });
            wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(clear.r),
                g: f64::from(clear.g),
                b: f64::from(clear.b),
                a: f64::from(clear.a),
            })
        }
    }
}

pub(super) fn store(op: AttachmentStoreOp) -> wgpu::StoreOp {
    match op {
        AttachmentStoreOp::Store => wgpu::StoreOp::Store,
        AttachmentStoreOp::DontCare => wgpu::StoreOp::Discard,
    }
}

pub(super) fn present_mode(mode: PresentMode) -> wgpu::PresentMode {
    match mode {
        PresentMode::Immediate => wgpu::PresentMode::Immediate,
        PresentMode::Mailbox => wgpu::PresentMode::Mailbox,
        PresentMode::Fifo => wgpu::PresentMode::Fifo,
        PresentMode::AutoVsync => wgpu::PresentMode::AutoVsync,
        PresentMode::AutoNoVsync => wgpu::PresentMode::AutoNoVsync,
        PresentMode::FifoRelaxed => wgpu::PresentMode::FifoRelaxed,
    }
}

/// The wgpu present mode matching a GAL request, or FIFO (always
/// supported) when the surface lacks it.
pub(super) fn supported_present_mode(
    mode: PresentMode,
    supported: &[wgpu::PresentMode],
) -> wgpu::PresentMode {
    let requested = present_mode(mode);
    if matches!(
        requested,
        wgpu::PresentMode::AutoVsync | wgpu::PresentMode::AutoNoVsync
    ) || supported.contains(&requested)
    {
        requested
    } else {
        wgpu::PresentMode::Fifo
    }
}

pub(super) fn checked_u32(value: u64, what: &str) -> GalResult<u32> {
    u32::try_from(value).map_err(|_| GalError::invalid_argument(format!("{what} exceeds u32")))
}
