use crate::configs::AgentOs;

/// Representing WebGPU hardware limit values used to spoof `navigator.gpu.requestAdapter().limits`.
///
/// These limits help simulate realistic platform-specific GPU profiles (e.g., macOS Metal vs. NVIDIA on Linux).
/// The values are injected into the adapter returned by WebGPU in spoofed environments.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct GpuLimits {
    /// Maximum dimension for 1D textures.
    pub max_texture_dimension_1d: u64,
    /// Maximum dimension for 2D textures.
    pub max_texture_dimension_2d: u64,
    /// Maximum dimension for 3D textures.
    pub max_texture_dimension_3d: u64,
    /// Maximum number of layers in texture arrays.
    pub max_texture_array_layers: u64,
    /// Maximum number of bind groups.
    pub max_bind_groups: u64,
    /// Maximum number of bind groups plus vertex buffers.
    pub max_bind_groups_plus_vertex_buffers: u64,
    /// Maximum bindings allowed per bind group.
    pub max_bindings_per_bind_group: u64,
    /// Maximum dynamic uniform buffers per pipeline layout.
    pub max_dynamic_uniform_buffers_per_pipeline_layout: u64,
    /// Maximum dynamic storage buffers per pipeline layout.
    pub max_dynamic_storage_buffers_per_pipeline_layout: u64,
    /// Maximum sampled textures per shader stage.
    pub max_sampled_textures_per_shader_stage: u64,
    /// Maximum samplers per shader stage.
    pub max_samplers_per_shader_stage: u64,
    /// Maximum storage buffers per shader stage.
    pub max_storage_buffers_per_shader_stage: u64,
    /// Maximum storage textures per shader stage.
    pub max_storage_textures_per_shader_stage: u64,
    /// Maximum uniform buffers per shader stage.
    pub max_uniform_buffers_per_shader_stage: u64,
    /// Maximum allowed size of a single uniform buffer binding.
    pub max_uniform_buffer_binding_size: u64,
    /// Maximum allowed size of a single storage buffer binding.
    pub max_storage_buffer_binding_size: u64,
    /// Minimum alignment of uniform buffer offset.
    pub min_uniform_buffer_offset_alignment: u64,
    /// Minimum alignment of storage buffer offset.
    pub min_storage_buffer_offset_alignment: u64,
    /// Maximum number of vertex buffers.
    pub max_vertex_buffers: u64,
    /// Maximum size of a single buffer.
    pub max_buffer_size: u64,
    /// Maximum number of vertex attributes.
    pub max_vertex_attributes: u64,
    /// Maximum allowed stride between vertex buffer entries.
    pub max_vertex_buffer_array_stride: u64,
    /// Maximum number of inter-stage shader variables.
    pub max_inter_stage_shader_variables: u64,
    /// Maximum number of color attachments.
    pub max_color_attachments: u64,
    /// Maximum total bytes per sample for color attachments.
    pub max_color_attachment_bytes_per_sample: u64,
    /// Maximum shared storage size for compute workgroups.
    pub max_compute_workgroup_storage_size: u64,
    /// Maximum invocations allowed per compute workgroup.
    pub max_compute_invocations_per_workgroup: u64,
    /// Maximum size of a compute workgroup in X dimension.
    pub max_compute_workgroup_size_x: u64,
    /// Maximum size of a compute workgroup in Y dimension.
    pub max_compute_workgroup_size_y: u64,
    /// Maximum size of a compute workgroup in Z dimension.
    pub max_compute_workgroup_size_z: u64,
    /// Maximum number of workgroups per dimension in compute shaders.
    pub max_compute_workgroups_per_dimension: u64,
}

impl Default for GpuLimits {
    /// standard mac m1 max gpu.
    fn default() -> Self {
        Self {
            max_texture_dimension_1d: 16384,
            max_texture_dimension_2d: 16384,
            max_texture_dimension_3d: 2048,
            max_texture_array_layers: 2048,
            max_bind_groups: 4,
            max_bind_groups_plus_vertex_buffers: 24,
            max_bindings_per_bind_group: 1000,
            max_dynamic_uniform_buffers_per_pipeline_layout: 10,
            max_dynamic_storage_buffers_per_pipeline_layout: 8,
            max_sampled_textures_per_shader_stage: 16,
            max_samplers_per_shader_stage: 16,
            max_storage_buffers_per_shader_stage: 10,
            max_storage_textures_per_shader_stage: 8,
            max_uniform_buffers_per_shader_stage: 12,
            max_uniform_buffer_binding_size: 65536,
            max_storage_buffer_binding_size: 4294967292,
            min_uniform_buffer_offset_alignment: 256,
            min_storage_buffer_offset_alignment: 256,
            max_vertex_buffers: 8,
            max_buffer_size: 4294967296,
            max_vertex_attributes: 30,
            max_vertex_buffer_array_stride: 2048,
            max_inter_stage_shader_variables: 28,
            max_color_attachments: 8,
            max_color_attachment_bytes_per_sample: 128,
            max_compute_workgroup_storage_size: 32768,
            max_compute_invocations_per_workgroup: 1024,
            max_compute_workgroup_size_x: 1024,
            max_compute_workgroup_size_y: 1024,
            max_compute_workgroup_size_z: 64,
            max_compute_workgroups_per_dimension: 65535,
        }
    }
}

impl GpuLimits {
    /// Returns a slightly jittered version of the limits, padded based on hardware_concurrency.
    pub fn with_variation(&self, hardware_concurrency: usize) -> Self {
        // Normalized range multiplier (1x for 2–4 cores, up to ~1.5x for 16+)
        let scale = (hardware_concurrency as f32 / 8.0).clamp(0.5, 1.5);

        let bump = |base: u64, jitter: u64| {
            let jitter_scaled = ((jitter as f32) * scale) as u64;
            base + rand::random_range(0..=jitter_scaled)
        };

        Self {
            max_bind_groups_plus_vertex_buffers: bump(self.max_bind_groups_plus_vertex_buffers, 2),
            max_bindings_per_bind_group: bump(self.max_bindings_per_bind_group, 20),
            max_dynamic_uniform_buffers_per_pipeline_layout: bump(
                self.max_dynamic_uniform_buffers_per_pipeline_layout,
                2,
            ),
            max_dynamic_storage_buffers_per_pipeline_layout: bump(
                self.max_dynamic_storage_buffers_per_pipeline_layout,
                2,
            ),
            max_sampled_textures_per_shader_stage: bump(
                self.max_sampled_textures_per_shader_stage,
                4,
            ),
            max_samplers_per_shader_stage: bump(self.max_samplers_per_shader_stage, 4),
            max_storage_buffers_per_shader_stage: bump(
                self.max_storage_buffers_per_shader_stage,
                2,
            ),
            max_storage_textures_per_shader_stage: bump(
                self.max_storage_textures_per_shader_stage,
                2,
            ),
            max_uniform_buffers_per_shader_stage: bump(
                self.max_uniform_buffers_per_shader_stage,
                2,
            ),
            max_uniform_buffer_binding_size: bump(self.max_uniform_buffer_binding_size, 4096),
            max_storage_buffer_binding_size: bump(self.max_storage_buffer_binding_size, 65536),
            max_vertex_attributes: bump(self.max_vertex_attributes, 4),
            max_inter_stage_shader_variables: bump(self.max_inter_stage_shader_variables, 2),
            max_color_attachment_bytes_per_sample: bump(
                self.max_color_attachment_bytes_per_sample,
                16,
            ),
            max_compute_workgroup_storage_size: bump(self.max_compute_workgroup_storage_size, 4096),
            max_compute_invocations_per_workgroup: bump(
                self.max_compute_invocations_per_workgroup,
                128,
            ),
            max_compute_workgroup_size_x: bump(self.max_compute_workgroup_size_x, 32),
            max_compute_workgroup_size_y: bump(self.max_compute_workgroup_size_y, 32),
            max_compute_workgroup_size_z: bump(self.max_compute_workgroup_size_z, 8),
            max_compute_workgroups_per_dimension: bump(
                self.max_compute_workgroups_per_dimension,
                1024,
            ),
            // Unchanged constants
            max_texture_dimension_1d: self.max_texture_dimension_1d,
            max_texture_dimension_2d: self.max_texture_dimension_2d,
            max_texture_dimension_3d: self.max_texture_dimension_3d,
            max_texture_array_layers: self.max_texture_array_layers,
            max_bind_groups: self.max_bind_groups,
            min_uniform_buffer_offset_alignment: self.min_uniform_buffer_offset_alignment,
            min_storage_buffer_offset_alignment: self.min_storage_buffer_offset_alignment,
            max_vertex_buffers: self.max_vertex_buffers,
            max_buffer_size: self.max_buffer_size,
            max_vertex_buffer_array_stride: self.max_vertex_buffer_array_stride,
            max_color_attachments: self.max_color_attachments,
        }
    }

    /// Get the GouLimit for the OS.
    pub fn for_os(os: AgentOs) -> Self {
        match os {
            AgentOs::Mac => Self {
                max_texture_dimension_1d: 16384,
                max_texture_dimension_2d: 16384,
                max_texture_dimension_3d: 2048,
                max_texture_array_layers: 2048,
                max_bind_groups: 4,
                max_bind_groups_plus_vertex_buffers: 24,
                max_bindings_per_bind_group: 1000,
                max_dynamic_uniform_buffers_per_pipeline_layout: 10,
                max_dynamic_storage_buffers_per_pipeline_layout: 8,
                max_sampled_textures_per_shader_stage: 16,
                max_samplers_per_shader_stage: 16,
                max_storage_buffers_per_shader_stage: 10,
                max_storage_textures_per_shader_stage: 8,
                max_uniform_buffers_per_shader_stage: 12,
                max_uniform_buffer_binding_size: 65536,
                max_storage_buffer_binding_size: 4294967292,
                min_uniform_buffer_offset_alignment: 256,
                min_storage_buffer_offset_alignment: 256,
                max_vertex_buffers: 8,
                max_buffer_size: 4294967296,
                max_vertex_attributes: 30,
                max_vertex_buffer_array_stride: 2048,
                max_inter_stage_shader_variables: 28,
                max_color_attachments: 8,
                max_color_attachment_bytes_per_sample: 128,
                max_compute_workgroup_storage_size: 32768,
                max_compute_invocations_per_workgroup: 1024,
                max_compute_workgroup_size_x: 1024,
                max_compute_workgroup_size_y: 1024,
                max_compute_workgroup_size_z: 64,
                max_compute_workgroups_per_dimension: 65535,
            },
            AgentOs::Linux | AgentOs::Windows | AgentOs::Android | AgentOs::Unknown => Self {
                max_texture_dimension_1d: 16384,
                max_texture_dimension_2d: 16384,
                max_texture_dimension_3d: 2048,
                max_texture_array_layers: 2048,
                max_bind_groups: 4,
                max_bind_groups_plus_vertex_buffers: 32,
                max_bindings_per_bind_group: 1000,
                max_dynamic_uniform_buffers_per_pipeline_layout: 12,
                max_dynamic_storage_buffers_per_pipeline_layout: 16,
                max_sampled_textures_per_shader_stage: 32,
                max_samplers_per_shader_stage: 32,
                max_storage_buffers_per_shader_stage: 12,
                max_storage_textures_per_shader_stage: 12,
                max_uniform_buffers_per_shader_stage: 16,
                max_uniform_buffer_binding_size: 131072,
                max_storage_buffer_binding_size: 1073741824,
                min_uniform_buffer_offset_alignment: 256,
                min_storage_buffer_offset_alignment: 256,
                max_vertex_buffers: 16,
                max_buffer_size: 4294967296,
                max_vertex_attributes: 16,
                max_vertex_buffer_array_stride: 2048,
                max_inter_stage_shader_variables: 32,
                max_color_attachments: 8,
                max_color_attachment_bytes_per_sample: 256,
                max_compute_workgroup_storage_size: 65536,
                max_compute_invocations_per_workgroup: 2048,
                max_compute_workgroup_size_x: 256,
                max_compute_workgroup_size_y: 256,
                max_compute_workgroup_size_z: 64,
                max_compute_workgroups_per_dimension: 131072,
            },
        }
    }
}

pub fn build_gpu_request_adapter_script_from_limits(
    vendor: &str,
    architecture: &str,
    device: &str,
    description: &str,
    limits: &GpuLimits,
) -> String {
    let info = format!(
        "vendor:'{}',architecture:'{}',device:'{}',description:'{}'",
        vendor, architecture, device, description
    );

    let limits_str = format!(
        "maxTextureDimension1D:{},maxTextureDimension2D:{},maxTextureDimension3D:{},maxTextureArrayLayers:{},maxBindGroups:{},maxBindGroupsPlusVertexBuffers:{},maxBindingsPerBindGroup:{},maxDynamicUniformBuffersPerPipelineLayout:{},maxDynamicStorageBuffersPerPipelineLayout:{},maxSampledTexturesPerShaderStage:{},maxSamplersPerShaderStage:{},maxStorageBuffersPerShaderStage:{},maxStorageTexturesPerShaderStage:{},maxUniformBuffersPerShaderStage:{},maxUniformBufferBindingSize:{},maxStorageBufferBindingSize:{},minUniformBufferOffsetAlignment:{},minStorageBufferOffsetAlignment:{},maxVertexBuffers:{},maxBufferSize:{},maxVertexAttributes:{},maxVertexBufferArrayStride:{},maxInterStageShaderVariables:{},maxColorAttachments:{},maxColorAttachmentBytesPerSample:{},maxComputeWorkgroupStorageSize:{},maxComputeInvocationsPerWorkgroup:{},maxComputeWorkgroupSizeX:{},maxComputeWorkgroupSizeY:{},maxComputeWorkgroupSizeZ:{},maxComputeWorkgroupsPerDimension:{}",
        limits.max_texture_dimension_1d,
        limits.max_texture_dimension_2d,
        limits.max_texture_dimension_3d,
        limits.max_texture_array_layers,
        limits.max_bind_groups,
        limits.max_bind_groups_plus_vertex_buffers,
        limits.max_bindings_per_bind_group,
        limits.max_dynamic_uniform_buffers_per_pipeline_layout,
        limits.max_dynamic_storage_buffers_per_pipeline_layout,
        limits.max_sampled_textures_per_shader_stage,
        limits.max_samplers_per_shader_stage,
        limits.max_storage_buffers_per_shader_stage,
        limits.max_storage_textures_per_shader_stage,
        limits.max_uniform_buffers_per_shader_stage,
        limits.max_uniform_buffer_binding_size,
        limits.max_storage_buffer_binding_size,
        limits.min_uniform_buffer_offset_alignment,
        limits.min_storage_buffer_offset_alignment,
        limits.max_vertex_buffers,
        limits.max_buffer_size,
        limits.max_vertex_attributes,
        limits.max_vertex_buffer_array_stride,
        limits.max_inter_stage_shader_variables,
        limits.max_color_attachments,
        limits.max_color_attachment_bytes_per_sample,
        limits.max_compute_workgroup_storage_size,
        limits.max_compute_invocations_per_workgroup,
        limits.max_compute_workgroup_size_x,
        limits.max_compute_workgroup_size_y,
        limits.max_compute_workgroup_size_z,
        limits.max_compute_workgroups_per_dimension,
    );

    format!(
        r#"(()=>{{const def=(o,m)=>Object.defineProperties(o,Object.fromEntries(Object.entries(m).map(([k,v])=>[k,{{value:v,enumerable:true,configurable:true}}]))),orig=navigator.gpu.requestAdapter.bind(navigator.gpu),I={{ {info} }},M={{ {limits_str} }};navigator.gpu.requestAdapter=async opts=>{{const a=await orig(opts),lim=a.limits;def(a.info,I);for(const k of Object.getOwnPropertyNames(lim))if(!(k in M))delete lim[k];def(lim,M);return a}}}})();"#
    )
}
