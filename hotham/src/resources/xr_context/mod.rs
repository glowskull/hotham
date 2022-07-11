use anyhow::Result;
use ash::vk::{self, Handle};
use openxr::{
    self as xr, EventDataBuffer, FrameStream, FrameWaiter, Session, SessionState, Space, Swapchain,
    Vulkan,
};
use xr::{
    vulkan::SessionCreateInfo, Duration, FrameState, ReferenceSpaceType, SwapchainCreateFlags,
    SwapchainCreateInfo, SwapchainUsageFlags, Time, View, ViewStateFlags,
};

use crate::{resources::VulkanContext, BLEND_MODE, COLOR_FORMAT, VIEW_COUNT, VIEW_TYPE};

mod input;
use input::Input;

#[derive(Default)]
pub struct XrContextBuilder<'a> {
    path: Option<&'a std::path::Path>,
    application_name: Option<&'a str>,
    application_version: Option<u32>,
    required_extensions: Option<xr::ExtensionSet>,
}

impl<'a> XrContextBuilder<'a> {
    pub fn new() -> Self {
        XrContextBuilder::default()
    }

    pub fn path(&mut self, path: Option<&'a std::path::Path>) -> &mut Self {
        self.path = path;
        self
    }

    pub fn application_name(&mut self, name: Option<&'a str>) -> &mut Self {
        self.application_name = name;
        self
    }

    pub fn application_version(&mut self, version: Option<u32>) -> &mut Self {
        self.application_version = version;
        self
    }

    pub fn required_extensions(&mut self, extensions: Option<xr::ExtensionSet>) -> &mut Self {
        self.required_extensions = extensions;
        self
    }

    pub fn build(&mut self) -> Result<(XrContext, VulkanContext)> {
        let application_name = self.application_name.unwrap_or("Hotham Application");
        let application_version = self.application_version.unwrap_or(1);
        let (instance, system) = create_xr_instance(
            self.path,
            application_name,
            application_version,
            self.required_extensions.as_ref(),
        )?;
        XrContext::_new(instance, system, application_name, application_version)
    }
}

pub struct XrContext {
    pub instance: openxr::Instance,
    pub session: Session<Vulkan>,
    pub session_state: SessionState,
    pub swapchain: Swapchain<Vulkan>,
    pub stage_space: Space,
    pub view_space: Space,
    pub input: Input,
    pub swapchain_resolution: vk::Extent2D,
    pub frame_waiter: FrameWaiter,
    pub frame_stream: FrameStream<Vulkan>,
    pub frame_state: FrameState,
    pub views: Vec<View>,
    pub view_state_flags: ViewStateFlags,
    pub frame_index: usize,
}

impl XrContext {
    pub fn new() -> Result<(XrContext, VulkanContext)> {
        XrContextBuilder::new().build()
    }

    pub fn new_from_path(path: &std::path::Path) -> Result<(XrContext, VulkanContext)> {
        XrContextBuilder::new().path(Some(path)).build()
    }

    #[cfg(test)]
    pub fn testing() -> (XrContext, VulkanContext) {
        XrContext::new_from_path(std::path::Path::new("../openxr_loader.dll")).unwrap()
    }

    fn _new(
        instance: xr::Instance,
        system: xr::SystemId,
        application_name: &str,
        application_version: u32,
    ) -> Result<(XrContext, VulkanContext)> {
        let vulkan_context =
            create_vulkan_context(&instance, system, application_name, application_version)?;
        let (session, frame_waiter, frame_stream) =
            create_xr_session(&instance, system, &vulkan_context)?;
        let stage_space =
            session.create_reference_space(ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)?;
        let view_space =
            session.create_reference_space(ReferenceSpaceType::VIEW, xr::Posef::IDENTITY)?;
        let swapchain_resolution = get_swapchain_resolution(&instance, system)?;
        let swapchain = create_xr_swapchain(&session, &swapchain_resolution, VIEW_COUNT)?;

        let input = Input::oculus_touch_controller(&instance, &session)?;

        let frame_state = FrameState {
            predicted_display_time: Time::from_nanos(0),
            predicted_display_period: Duration::from_nanos(0),
            should_render: false,
        };

        // Attach the action set to the session
        session.attach_action_sets(&[&input.action_set])?;

        let xr_context = XrContext {
            instance,
            session,
            session_state: SessionState::IDLE,
            swapchain,
            stage_space,
            view_space,
            input,
            swapchain_resolution,
            frame_waiter,
            frame_stream,
            frame_state,
            views: Vec::new(),
            view_state_flags: ViewStateFlags::EMPTY,
            frame_index: 0,
        };

        Ok((xr_context, vulkan_context))
    }

    pub(crate) fn poll_xr_event(
        &mut self,
        event_buffer: &mut EventDataBuffer,
    ) -> Result<SessionState> {
        loop {
            match self.instance.poll_event(event_buffer)? {
                Some(xr::Event::SessionStateChanged(session_changed)) => {
                    let new_state = session_changed.state();
                    println!("[HOTHAM_POLL_EVENT] State is now {:?}", new_state);
                    self.session_state = new_state;
                }
                Some(xr::Event::InstanceLossPending(_)) => {
                    println!("[HOTHAM_POLL_EVENT] Instance loss pending!");
                    break;
                }
                Some(_) => println!("[HOTHAM_POLL_EVENT] Received some other event"),
                None => break,
            }
        }

        Ok(self.session_state)
    }

    pub(crate) fn begin_frame(&mut self) -> Result<()> {
        self.frame_state = self.frame_waiter.wait()?;
        self.frame_stream.begin()?;

        self.frame_index = self.swapchain.acquire_image()? as _;
        self.swapchain.wait_image(openxr::Duration::INFINITE)?;

        Ok(())
    }

    pub fn end_frame(&mut self) -> std::result::Result<(), openxr::sys::Result> {
        // Submit the image to OpenXR
        self.swapchain.release_image().unwrap();

        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: self.swapchain_resolution.width as _,
                height: self.swapchain_resolution.height as _,
            },
        };

        let display_time = self.frame_state.predicted_display_time;

        let views = [
            xr::CompositionLayerProjectionView::new()
                .pose(self.views[0].pose)
                .fov(self.views[0].fov)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&self.swapchain)
                        .image_array_index(0)
                        .image_rect(rect),
                ),
            xr::CompositionLayerProjectionView::new()
                .pose(self.views[1].pose)
                .fov(self.views[1].fov)
                .sub_image(
                    xr::SwapchainSubImage::new()
                        .swapchain(&self.swapchain)
                        .image_array_index(1)
                        .image_rect(rect),
                ),
        ];

        let layer_projection = xr::CompositionLayerProjection::new()
            .space(&self.stage_space)
            .views(&views);

        let layers = [&*layer_projection];
        self.frame_stream.end(display_time, BLEND_MODE, &layers)
    }

    pub(crate) fn end_session(&mut self) -> anyhow::Result<()> {
        println!("[HOTHAM_XR] - Ending session..");
        self.session.end()?;
        println!("[HOTHAM_XR] - ..done!");
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) fn create_vulkan_context(
    xr_instance: &xr::Instance,
    system: xr::SystemId,
    application_name: &str,
    application_version: u32,
) -> Result<VulkanContext, crate::hotham_error::HothamError> {
    let vulkan_context = VulkanContext::create_from_xr_instance_legacy(
        xr_instance,
        system,
        application_name,
        application_version,
    )?;
    println!("[HOTHAM_VULKAN] - Vulkan Context created successfully");
    Ok(vulkan_context)
}

#[cfg(target_os = "android")]
fn create_vulkan_context(
    xr_instance: &xr::Instance,
    system: xr::SystemId,
    application_name: &str,
    application_version: u32,
) -> Result<VulkanContext, crate::hotham_error::HothamError> {
    let vulkan_context = VulkanContext::create_from_xr_instance_legacy(
        xr_instance,
        system,
        application_name,
        application_version,
    )?;
    println!("[HOTHAM_VULKAN] - Vulkan Context created successfully");
    Ok(vulkan_context)
}

pub(crate) fn get_swapchain_resolution(
    xr_instance: &xr::Instance,
    system: xr::SystemId,
) -> Result<vk::Extent2D> {
    let views = xr_instance.enumerate_view_configuration_views(system, VIEW_TYPE)?;
    println!("[HOTHAM_VULKAN] Views: {:?}", views);
    let resolution = vk::Extent2D {
        width: views[0].recommended_image_rect_width,
        height: views[0].recommended_image_rect_height,
    };

    Ok(resolution)
}

pub(crate) fn create_xr_swapchain(
    xr_session: &Session<Vulkan>,
    resolution: &vk::Extent2D,
    array_size: u32,
) -> Result<Swapchain<Vulkan>> {
    xr_session
        .create_swapchain(&SwapchainCreateInfo {
            create_flags: SwapchainCreateFlags::EMPTY,
            usage_flags: SwapchainUsageFlags::COLOR_ATTACHMENT,
            format: COLOR_FORMAT.as_raw() as u32,
            sample_count: 1,
            width: resolution.width,
            height: resolution.height,
            face_count: 1,
            array_size,
            mip_count: 1,
        })
        .map_err(Into::into)
}

pub(crate) fn create_xr_session(
    xr_instance: &xr::Instance,
    system: xr::SystemId,
    vulkan_context: &VulkanContext,
) -> Result<(Session<Vulkan>, FrameWaiter, FrameStream<Vulkan>)> {
    println!("[HOTHAM] Creating session..");
    Ok(unsafe {
        xr_instance.create_session(
            system,
            &SessionCreateInfo {
                instance: vulkan_context.instance.handle().as_raw() as *const _,
                physical_device: vulkan_context.physical_device.as_raw() as *const _,
                device: vulkan_context.device.handle().as_raw() as *const _,
                queue_family_index: vulkan_context.queue_family_index,
                queue_index: 0,
            },
        )
    }
    .unwrap())
}

pub(crate) fn create_xr_instance(
    path: Option<&std::path::Path>,
    application_name: &str,
    application_version: u32,
    required_extensions: Option<&xr::ExtensionSet>,
) -> anyhow::Result<(xr::Instance, xr::SystemId)> {
    let xr_entry = if let Some(path) = path {
        xr::Entry::load_from(path)?
    } else {
        xr::Entry::load()?
    };
    let xr_app_info = openxr::ApplicationInfo {
        application_name,
        application_version,
        engine_name: "Hotham",
        engine_version: 1,
    };
    let mut required_extensions = required_extensions.cloned().unwrap_or_default();
    // required_extensions.khr_vulkan_enable2 = true; // TODO: Should we use enable 2 for the simulator..?
    required_extensions.khr_vulkan_enable = true; // TODO: Should we use enable 2 for the simulator..?

    #[cfg(target_os = "android")]
    {
        required_extensions.khr_android_create_instance = true;
        xr_entry.initialize_android_loader()?;
    }

    println!(
        "Available extensions: {:?}",
        xr_entry.enumerate_extensions()?
    );

    let instance = xr_entry.create_instance(&xr_app_info, &required_extensions, &[])?;
    let system = instance.system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)?;
    Ok((instance, system))
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod tests {
    use super::XrContext;

    #[test]
    pub fn test_xr_context_smoke_test() {
        XrContext::testing();
    }
}