use crate::client::TelegramClient;
use crate::net::{
    AddContactHttpRequest, AddContactHttpResponse, AuthCustodyUpdateResponse, AuthResponse,
    BlockedUserDto, ChangePasswordRequest, ContactDto, EmailRegisterRequest,
    EmailSendCodeResponse, HttpClientError, KdfParamsResponse, LoginRequest, PhoneSendCodeRequest,
    PhoneSendCodeResponse, PhoneVerifyRequest, ProtoAuthResponse, ProtoUpdateProfileRequest,
    QrPairInitResponse, QrPairPollResponse, RegisterRequest, SessionDto,
    SplitKeyEmailRegisterRequest, SplitKeyLoginRequest, SplitKeyRegisterRequest, StatusResponse,
    TwoFactorSetRequest, TwoFactorStatusResponse, TwoFactorVerifyRequest, UpgradeSplitKeyRequest,
    UserMeResponse, UserPrivacySettingsDto, UserProfileDto,
};

pub fn auth_from_proto(p: ProtoAuthResponse) -> AuthResponse {
    AuthResponse::from(p)
}

impl TelegramClient {
    pub async fn check_health(&self) -> std::result::Result<bool, HttpClientError> {
        self.http_client.check_health().await
    }

    pub async fn fetch_server_status(&self) -> std::result::Result<StatusResponse, HttpClientError> {
        self.http_client.fetch_server_status().await
    }

    pub async fn login(&self, req: &LoginRequest) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = self.http_client.login(req).await?;
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub async fn register(
        &self,
        req: &RegisterRequest,
    ) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = self.http_client.register(req).await?;
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub fn logout(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.clear_session();
            self.http_client.logout(&token).await
        }
    }

    pub async fn phone_send_code(
        &self,
        phone: &str,
    ) -> std::result::Result<PhoneSendCodeResponse, HttpClientError> {
        let req = PhoneSendCodeRequest {
            phone: phone.to_string(),
        };
        self.http_client.phone_send_code(&req).await
    }

    pub async fn phone_verify(
        &self,
        req: &PhoneVerifyRequest,
    ) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = auth_from_proto(self.http_client.phone_verify(req).await?);
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub async fn email_send_code(
        &self,
        email: &str,
    ) -> std::result::Result<EmailSendCodeResponse, HttpClientError> {
        self.http_client.email_send_code(email).await
    }

    pub async fn email_register(
        &self,
        req: &EmailRegisterRequest,
    ) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = self.http_client.email_register(req).await?;
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub async fn get_auth_kdf_params(
        &self,
        username_or_email: &str,
    ) -> std::result::Result<KdfParamsResponse, HttpClientError> {
        self.http_client.get_auth_kdf_params(username_or_email).await
    }

    pub async fn register_split_key(
        &self,
        req: &SplitKeyRegisterRequest,
    ) -> std::result::Result<ProtoAuthResponse, HttpClientError> {
        let auth = self.http_client.register_split_key(req).await?;
        self.set_auth_session(auth_from_proto(auth.clone()));
        Ok(auth)
    }

    pub async fn email_register_split_key(
        &self,
        req: &SplitKeyEmailRegisterRequest,
    ) -> std::result::Result<ProtoAuthResponse, HttpClientError> {
        let auth = self.http_client.email_register_split_key(req).await?;
        self.set_auth_session(auth_from_proto(auth.clone()));
        Ok(auth)
    }

    pub async fn login_split_key(
        &self,
        req: &SplitKeyLoginRequest,
    ) -> std::result::Result<ProtoAuthResponse, HttpClientError> {
        let auth = self.http_client.login_split_key(req).await?;
        self.set_auth_session(auth_from_proto(auth.clone()));
        Ok(auth)
    }

    pub fn upgrade_split_key<'a>(
        &'a self,
        req: &'a UpgradeSplitKeyRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<AuthCustodyUpdateResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.upgrade_split_key_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn upgrade_split_key_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &UpgradeSplitKeyRequest,
    ) -> std::result::Result<AuthCustodyUpdateResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.upgrade_split_key(token, req).await
    }

    pub fn change_password<'a>(
        &'a self,
        req: &'a ChangePasswordRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<AuthCustodyUpdateResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.change_password(&token, req).await
        }
    }

    pub async fn email_verify(
        &self,
        email: &str,
        code: &str,
    ) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = auth_from_proto(self.http_client.email_verify(email, code).await?);
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub async fn qr_pair_init(&self) -> std::result::Result<QrPairInitResponse, HttpClientError> {
        self.http_client.qr_pair_init().await
    }

    pub async fn qr_pair_poll(
        &self,
        pair_token: &str,
    ) -> std::result::Result<QrPairPollResponse, HttpClientError> {
        let res = self.http_client.qr_pair_poll(pair_token).await?;
        if let Some(ref sess) = res.session {
            self.set_auth_session(auth_from_proto(sess.clone()));
        } else if let Some(ref t) = res.token {
            self.set_token(Some(t.clone()));
        }
        Ok(res)
    }

    pub fn qr_pair_accept<'a>(
        &'a self,
        pair_token: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.qr_pair_accept(&token, pair_token).await
        }
    }

    pub async fn qr_pair_verify(
        &self,
        pair_token: &str,
        cloud_password: Option<&str>,
    ) -> std::result::Result<QrPairPollResponse, HttpClientError> {
        let res = self.http_client.qr_pair_verify(pair_token, cloud_password).await?;
        if let Some(ref sess) = res.session {
            self.set_auth_session(auth_from_proto(sess.clone()));
        } else if let Some(ref t) = res.token {
            self.set_token(Some(t.clone()));
        }
        Ok(res)
    }

    pub fn get_2fa_status(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<TwoFactorStatusResponse, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_2fa_status_with_token(&token, expected_epoch).await
        }
    }

    pub async fn get_2fa_status_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<TwoFactorStatusResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_2fa_status(token).await
    }

    pub fn set_2fa<'a>(
        &'a self,
        req: &'a TwoFactorSetRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<TwoFactorStatusResponse, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.set_2fa_with_token(&token, expected_epoch, req).await
        }
    }

    pub async fn set_2fa_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &TwoFactorSetRequest,
    ) -> std::result::Result<TwoFactorStatusResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.set_2fa(token, req).await
    }

    pub fn disable_2fa<'a>(
        &'a self,
        current_password: Option<&'a str>,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.disable_2fa_with_token(&token, expected_epoch, current_password)
                .await
        }
    }

    pub async fn disable_2fa_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        current_password: Option<&str>,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.disable_2fa(token, current_password).await
    }

    pub async fn verify_2fa(
        &self,
        req: &TwoFactorVerifyRequest,
    ) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = auth_from_proto(self.http_client.verify_2fa(req).await?);
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub async fn passkey_challenge(
        &self,
        username: Option<&str>,
    ) -> std::result::Result<serde_json::Value, HttpClientError> {
        self.http_client.passkey_challenge(username).await
    }

    pub async fn passkey_verify(
        &self,
        credential: &serde_json::Value,
    ) -> std::result::Result<AuthResponse, HttpClientError> {
        let auth = auth_from_proto(self.http_client.passkey_verify(credential).await?);
        self.set_auth_session(auth.clone());
        Ok(auth)
    }

    pub fn list_passkeys(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<serde_json::Value>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.list_passkeys(&token).await
        }
    }

    pub fn register_passkey<'a>(
        &'a self,
        body: &'a serde_json::Value,
    ) -> impl std::future::Future<Output = std::result::Result<serde_json::Value, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.register_passkey(&token, body).await
        }
    }

    pub fn delete_passkey<'a>(
        &'a self,
        passkey_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.delete_passkey(&token, passkey_id).await
        }
    }

    pub fn get_me(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<UserMeResponse, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.get_me(&token).await
        }
    }

    pub fn list_sessions(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<SessionDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_sessions_with_token(&token, expected_epoch).await
        }
    }

    pub async fn list_sessions_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<SessionDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_sessions(token).await
    }

    pub fn revoke_session<'a>(
        &'a self,
        session_id: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<bool, HttpClientError>> + Send + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.revoke_session_with_token(&token, expected_epoch, session_id)
                .await
        }
    }

    pub async fn revoke_session_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        session_id: &str,
    ) -> std::result::Result<bool, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.revoke_session(token, session_id).await
    }

    pub fn update_profile<'a>(
        &'a self,
        req: &'a ProtoUpdateProfileRequest,
    ) -> impl std::future::Future<Output = std::result::Result<UserProfileDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.update_profile_with_token(&token, expected_epoch, req)
                .await
        }
    }

    pub async fn update_profile_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        req: &ProtoUpdateProfileRequest,
    ) -> std::result::Result<UserProfileDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.update_user_profile(token, req).await
    }

    pub fn get_user_profile(
        &self,
        user_id: u64,
    ) -> impl std::future::Future<Output = std::result::Result<UserProfileDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_user_profile_with_token(&token, expected_epoch, user_id)
                .await
        }
    }

    pub async fn get_user_profile_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        user_id: u64,
    ) -> std::result::Result<UserProfileDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_user_profile(token, user_id).await
    }

    pub fn resolve_peer<'a>(
        &'a self,
        username: &'a str,
    ) -> impl std::future::Future<Output = std::result::Result<UserProfileDto, HttpClientError>>
           + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.resolve_peer(&token, username).await
        }
    }

    pub fn list_contacts(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<ContactDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_contacts_with_token(&token, expected_epoch).await
        }
    }

    pub async fn list_contacts_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<ContactDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_contacts(token).await
    }

    pub fn add_contact(
        &self,
        payload: AddContactHttpRequest,
    ) -> impl std::future::Future<
        Output = std::result::Result<AddContactHttpResponse, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.add_contact_with_token(&token, expected_epoch, payload)
                .await
        }
    }

    pub async fn add_contact_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        payload: AddContactHttpRequest,
    ) -> std::result::Result<AddContactHttpResponse, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.add_contact(token, payload).await
    }

    pub fn delete_contact(
        &self,
        user_id: u64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.ensure_pinned_session(expected_epoch, &token)?;
            self.http_client.delete_contact(&token, user_id).await
        }
    }

    pub fn get_my_profile(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<UserProfileDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_my_profile_with_token(&token, expected_epoch).await
        }
    }

    pub async fn get_my_profile_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<UserProfileDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_my_profile(token).await
    }

    pub fn revoke_other_sessions(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.revoke_other_sessions_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn revoke_other_sessions_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.revoke_other_sessions(token).await
    }

    pub fn list_blocked_users(
        &self,
    ) -> impl std::future::Future<Output = std::result::Result<Vec<BlockedUserDto>, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.list_blocked_users_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn list_blocked_users_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<BlockedUserDto>, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.list_blocked_users(token).await
    }

    pub fn block_user(
        &self,
        user_id: u64,
    ) -> impl std::future::Future<Output = std::result::Result<BlockedUserDto, HttpClientError>>
           + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.block_user_with_token(&token, expected_epoch, user_id)
                .await
        }
    }

    pub async fn block_user_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        user_id: u64,
    ) -> std::result::Result<BlockedUserDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.block_user(token, user_id).await
    }

    pub fn unblock_user(
        &self,
        user_id: u64,
    ) -> impl std::future::Future<Output = std::result::Result<(), HttpClientError>> + Send + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.unblock_user_with_token(&token, expected_epoch, user_id)
                .await
        }
    }

    pub async fn unblock_user_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        user_id: u64,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.unblock_user(token, user_id).await
    }

    pub fn get_my_settings(
        &self,
    ) -> impl std::future::Future<
        Output = std::result::Result<UserPrivacySettingsDto, HttpClientError>,
    > + Send
           + '_ {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.get_my_settings_with_token(&token, expected_epoch)
                .await
        }
    }

    pub async fn get_my_settings_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<UserPrivacySettingsDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.get_my_settings(token).await
    }

    pub fn save_my_settings<'a>(
        &'a self,
        settings: &'a UserPrivacySettingsDto,
    ) -> impl std::future::Future<
        Output = std::result::Result<UserPrivacySettingsDto, HttpClientError>,
    > + Send
           + 'a {
        let pinned = self.snapshot_pinned_session();
        async move {
            let (expected_epoch, token) = pinned?;
            self.save_my_settings_with_token(&token, expected_epoch, settings)
                .await
        }
    }

    pub async fn save_my_settings_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        settings: &UserPrivacySettingsDto,
    ) -> std::result::Result<UserPrivacySettingsDto, HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        self.http_client.save_my_settings(token, settings).await
    }

    pub async fn upsert_proxy(
        &self,
        proxy: crate::session_persistence::ProxyEndpointRecord,
    ) -> std::result::Result<(), HttpClientError> {
        let _ = crate::net::proxy::ClientProxyConfig::validate_record(&proxy)?;
        let mut state = crate::session_persistence::SessionPersistenceBridge::load_state_from_disk();
        if proxy.enabled {
            for existing in &mut state.proxies {
                existing.enabled = false;
            }
        }
        if let Some(slot) = state.proxies.iter_mut().find(|p| p.id == proxy.id) {
            *slot = proxy;
        } else {
            state.proxies.push(proxy);
        }
        let active_proxy_rec = state
            .proxies
            .iter()
            .find(|p| p.enabled && !p.kind.eq_ignore_ascii_case("Direct"))
            .or_else(|| state.proxies.iter().find(|p| p.enabled));
        let active_cfg = match active_proxy_rec {
            Some(rec) => crate::net::proxy::ClientProxyConfig::validate_record(rec)?,
            None => None,
        };
        self.apply_proxy(active_cfg)?;
        crate::session_persistence::SessionPersistenceBridge::save_state_to_disk(&state);
        Ok(())
    }

    pub async fn list_proxies(
        &self,
    ) -> std::result::Result<Vec<crate::session_persistence::ProxyEndpointRecord>, HttpClientError>
    {
        let state = crate::session_persistence::SessionPersistenceBridge::load_state_from_disk();
        Ok(state.proxies)
    }

    pub async fn upsert_proxy_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
        proxy: crate::session_persistence::ProxyEndpointRecord,
    ) -> std::result::Result<(), HttpClientError> {
        self.ensure_pinned_session(expected_epoch, token)?;
        let _ = crate::net::proxy::ClientProxyConfig::validate_record(&proxy)?;
        let mut state = crate::session_persistence::SessionPersistenceBridge::load_state_from_disk();
        if proxy.enabled {
            for existing in &mut state.proxies {
                existing.enabled = false;
            }
        }
        if let Some(slot) = state.proxies.iter_mut().find(|p| p.id == proxy.id) {
            *slot = proxy;
        } else {
            state.proxies.push(proxy);
        }
        let active_proxy_rec = state
            .proxies
            .iter()
            .find(|p| p.enabled && !p.kind.eq_ignore_ascii_case("Direct"))
            .or_else(|| state.proxies.iter().find(|p| p.enabled));
        let active_cfg = match active_proxy_rec {
            Some(rec) => crate::net::proxy::ClientProxyConfig::validate_record(rec)?,
            None => None,
        };
        if !self.is_same_session_epoch(expected_epoch) {
            return Err(HttpClientError::Unauthorized(
                "Session changed before saving proxy".to_string(),
            ));
        }
        self.apply_proxy(active_cfg)?;
        crate::session_persistence::SessionPersistenceBridge::save_state_to_disk(&state);
        Ok(())
    }

    pub async fn list_proxies_with_token(
        &self,
        token: &str,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<crate::session_persistence::ProxyEndpointRecord>, HttpClientError>
    {
        self.ensure_pinned_session(expected_epoch, token)?;
        let state = crate::session_persistence::SessionPersistenceBridge::load_state_from_disk();
        Ok(state.proxies)
    }

    pub async fn upsert_proxy_for_session(
        &self,
        token: Option<&str>,
        expected_epoch: u64,
        proxy: crate::session_persistence::ProxyEndpointRecord,
    ) -> std::result::Result<(), HttpClientError> {
        match token {
            Some(tok) => self.upsert_proxy_with_token(tok, expected_epoch, proxy).await,
            None => {
                if !self.is_same_session_epoch(expected_epoch)
                    || self.token().is_some_and(|cur| !cur.trim().is_empty())
                {
                    return Err(HttpClientError::Unauthorized(
                        "Session changed before saving proxy".to_string(),
                    ));
                }
                self.upsert_proxy(proxy).await
            }
        }
    }

    pub async fn list_proxies_for_session(
        &self,
        token: Option<&str>,
        expected_epoch: u64,
    ) -> std::result::Result<Vec<crate::session_persistence::ProxyEndpointRecord>, HttpClientError>
    {
        match token {
            Some(tok) => self.list_proxies_with_token(tok, expected_epoch).await,
            None => {
                if !self.is_same_session_epoch(expected_epoch)
                    || self.token().is_some_and(|cur| !cur.trim().is_empty())
                {
                    return Err(HttpClientError::Unauthorized(
                        "Session changed before listing proxies".to_string(),
                    ));
                }
                self.list_proxies().await
            }
        }
    }
}
