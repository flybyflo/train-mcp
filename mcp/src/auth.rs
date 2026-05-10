use axum::http::{header, HeaderMap, StatusCode};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone)]
pub struct AuthService {
    config: Option<AuthConfig>,
}

#[derive(Clone)]
struct AuthConfig {
    decoding_key: DecodingKey,
    issuer: Option<String>,
    audiences: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub session_key: String,
    pub sub: String,
    pub iss: Option<String>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub jti: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AuthContext {
    Anonymous,
    Authenticated(AuthenticatedUser),
}

impl AuthContext {
    pub fn user(&self) -> Option<&AuthenticatedUser> {
        match self {
            AuthContext::Anonymous => None,
            AuthContext::Authenticated(user) => Some(user),
        }
    }
}

#[derive(Debug)]
pub enum AuthError {
    InvalidAuthorizationHeader,
    AuthNotConfigured,
    InvalidToken(String),
}

impl AuthError {
    pub fn status_code(&self) -> StatusCode {
        StatusCode::UNAUTHORIZED
    }

    pub fn payload(&self) -> serde_json::Value {
        match self {
            AuthError::InvalidAuthorizationHeader => json!({
                "ok": false,
                "error": "invalid_authorization_header",
                "message": "Authorization header must be in the form: Bearer <token>.",
            }),
            AuthError::AuthNotConfigured => json!({
                "ok": false,
                "error": "auth_not_configured",
                "message": "JWT auth is not configured on this server.",
            }),
            AuthError::InvalidToken(message) => json!({
                "ok": false,
                "error": "invalid_token",
                "message": message,
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct JwtClaims {
    sub: String,
    iss: Option<String>,
    #[serde(rename = "aud")]
    _aud: Option<serde_json::Value>,
    #[serde(rename = "exp")]
    _exp: usize,
    #[serde(rename = "iat")]
    _iat: Option<usize>,
    email: Option<String>,
    name: Option<String>,
    jti: Option<String>,
}

impl AuthService {
    pub fn from_env() -> Self {
        let secret = std::env::var("AUTH_JWT_HS256_SECRET")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let issuer = std::env::var("AUTH_JWT_ISSUER")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let audiences = std::env::var("AUTH_JWT_AUDIENCE")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let config = secret.map(|secret| AuthConfig {
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            issuer,
            audiences,
        });
        Self { config }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
    }

    pub fn resolve(&self, headers: &HeaderMap) -> Result<AuthContext, AuthError> {
        let Some(raw) = headers.get(header::AUTHORIZATION) else {
            return Ok(AuthContext::Anonymous);
        };
        let Ok(raw) = raw.to_str() else {
            return Err(AuthError::InvalidAuthorizationHeader);
        };
        let token = extract_bearer_token(raw)?;

        let Some(config) = &self.config else {
            return Err(AuthError::AuthNotConfigured);
        };

        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        if let Some(issuer) = &config.issuer {
            validation.set_issuer(&[issuer]);
        }
        if !config.audiences.is_empty() {
            validation.set_audience(&config.audiences);
        }

        let decoded = decode::<JwtClaims>(token, &config.decoding_key, &validation)
            .map_err(|e| AuthError::InvalidToken(e.to_string()))?;
        let claims = decoded.claims;
        let sub = claims.sub.trim();
        if sub.is_empty() {
            return Err(AuthError::InvalidToken(
                "JWT claim \"sub\" cannot be empty.".into(),
            ));
        }
        let issuer = claims
            .iss
            .clone()
            .unwrap_or_else(|| "unknown_issuer".into());
        let user_id = format!("{issuer}:{sub}");
        let session_suffix = claims.jti.clone().unwrap_or_else(|| "default".into());
        let session_key = format!("{user_id}:{session_suffix}");

        Ok(AuthContext::Authenticated(AuthenticatedUser {
            user_id,
            session_key,
            sub: sub.to_string(),
            iss: claims.iss,
            email: claims.email,
            name: claims.name,
            jti: claims.jti,
        }))
    }
}

fn extract_bearer_token(value: &str) -> Result<&str, AuthError> {
    let mut parts = value.splitn(2, ' ');
    let scheme = parts.next().unwrap_or_default();
    let token = parts.next().unwrap_or_default().trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return Err(AuthError::InvalidAuthorizationHeader);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;

    use super::*;

    #[derive(Serialize)]
    struct Claims<'a> {
        sub: &'a str,
        iss: &'a str,
        aud: &'a str,
        exp: usize,
        iat: usize,
        email: &'a str,
        name: &'a str,
        jti: &'a str,
    }

    fn make_token(secret: &str) -> String {
        let claims = Claims {
            sub: "user-123",
            iss: "train-mcp-tests",
            aud: "train-mcp-clients",
            exp: usize::MAX / 4,
            iat: 1,
            email: "test@example.com",
            name: "Test User",
            jti: "sess-1",
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .expect("encode token")
    }

    #[test]
    fn resolve_without_header_is_anonymous() {
        let service = AuthService { config: None };
        let headers = HeaderMap::new();
        let context = service.resolve(&headers).expect("resolve");
        assert!(matches!(context, AuthContext::Anonymous));
    }

    #[test]
    fn resolve_valid_token_is_authenticated() {
        let secret = "super-secret";
        let service = AuthService {
            config: Some(AuthConfig {
                decoding_key: DecodingKey::from_secret(secret.as_bytes()),
                issuer: Some("train-mcp-tests".into()),
                audiences: vec!["train-mcp-clients".into()],
            }),
        };
        let token = make_token(secret);

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
        );

        let context = service.resolve(&headers).expect("resolve");
        let user = context.user().expect("user");
        assert_eq!(user.sub, "user-123");
        assert_eq!(user.user_id, "train-mcp-tests:user-123");
    }

    #[test]
    fn resolve_invalid_token_returns_error() {
        let service = AuthService {
            config: Some(AuthConfig {
                decoding_key: DecodingKey::from_secret("correct".as_bytes()),
                issuer: None,
                audiences: vec![],
            }),
        };
        let token = make_token("incorrect");

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
        );
        let result = service.resolve(&headers);
        assert!(matches!(result, Err(AuthError::InvalidToken(_))));
    }
}
