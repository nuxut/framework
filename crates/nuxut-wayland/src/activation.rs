//! Etkinleştirme belirteci (xdg-activation-v1): başlatıcı compositor'dan belirteç alıp
//! `XDG_ACTIVATION_TOKEN` ile çocuğa verir. Belirteç ve `app_id` hiçbir seviyede loglanmaz
//! (değişmez 9); `ActivationToken` kopyalanamaz, Debug'ı redakte.

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};
use crate::types::SurfaceId;

/// `request_activation_token` isteğinin kimliği; sonuç `App::activation_token` ile gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActivationRequestId(pub(crate) u64);

/// Compositor'ın verdiği belirteç. Clone yok (tek kullanımlık; kopyası sızmaz), Debug redakte.
pub struct ActivationToken(String);

impl ActivationToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Debug for ActivationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActivationToken(<gizli>)")
    }
}

impl<A: App> Context<A> {
    /// Etkinleştirme belirteci ister. `surface`, son girdinin (tuş ya da işaretçi basma)
    /// serial'i ve koltuğu runtime tarafından konur; `app_id` verilirse iletilir. Sonuç her
    /// istek için tam bir kez `App::activation_token` ile, bu çağrının içinden değil, gelir.
    /// Protokol yoksa, yüzey yoksa ya da oturum kilidi istenmiş/kuruluysa istek gönderilmez
    /// ve sonuç `None` olur.
    pub fn request_activation_token(
        &mut self,
        surface: SurfaceId,
        app_id: Option<&str>,
    ) -> ActivationRequestId {
        let id = ActivationRequestId(self.next_id());
        let (Some(manager), Some(entry), None) = (
            self.protocols.activation.as_ref(),
            self.surfaces.get(&surface),
            self.lock.as_ref(),
        ) else {
            tracing::debug!(
                id = id.0,
                "etkinleştirme belirteci istenmedi (protokol/yüzey/kilit)"
            );
            self.pending.push(RuntimeEvent::ActivationToken(id, None));
            return id;
        };
        let token = manager.get_activation_token(&self.qh, TokenData(id));
        if let Some(app_id) = app_id {
            token.set_app_id(app_id.to_owned());
        }
        if let Some((seat, serial)) = &self.last_input {
            token.set_serial(*serial, seat);
        }
        token.set_surface(entry.role.wl_surface());
        token.commit();
        tracing::debug!(id = id.0, "etkinleştirme belirteci istendi");
        id
    }
}

/// Belirteç nesnesinin verisi: hangi isteğe ait olduğu.
#[derive(Debug)]
pub(crate) struct TokenData(ActivationRequestId);

impl<A: App> Dispatch2<XdgActivationTokenV1, State<A>> for TokenData {
    fn event(
        &self,
        state: &mut State<A>,
        proxy: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            // `done` tek olay; nesne artık işe yaramaz.
            proxy.destroy();
            tracing::debug!(id = self.0.0, "etkinleştirme belirteci geldi");
            state.cx.pending.push(RuntimeEvent::ActivationToken(
                self.0,
                Some(ActivationToken(token)),
            ));
        }
    }
}

/// `xdg_activation_v1`'in olayı yok.
impl<D> Dispatch2<XdgActivationV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &XdgActivationV1,
        _: <XdgActivationV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_debug_is_redacted() {
        let token = ActivationToken("gizli-belirtec-123".into());
        let shown = format!("{token:?}");
        assert!(!shown.contains("gizli-belirtec"), "{shown}");
        assert_eq!(token.as_str(), "gizli-belirtec-123");
        assert_eq!(token.into_string(), "gizli-belirtec-123");
    }
}
