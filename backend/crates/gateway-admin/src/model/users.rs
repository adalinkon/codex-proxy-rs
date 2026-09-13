//! 用户身份、直接绑定的额度与授权配置。

use gateway_core::{
    engine::budget::{ClientBudgetLimits, ClientBudgetStatus},
    policy::RateLimits,
    routing::AccountGroupId,
};

use super::account_groups::AccountGroupRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRole {
    Admin,
    User,
}

impl UserRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserIdentity {
    pub id: String,
    pub role: UserRole,
    pub enabled: bool,
    pub auth_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub identity: UserIdentity,
    pub all_groups: bool,
    pub groups: Vec<AccountGroupRef>,
    pub limits: RateLimits,
    pub budget: ClientBudgetStatus,
    pub key_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPolicyUpdate {
    pub id: String,
    pub role: UserRole,
    pub enabled: bool,
    pub all_groups: bool,
    pub group_ids: Vec<AccountGroupId>,
    pub limits: RateLimits,
    pub budget: ClientBudgetLimits,
}

#[derive(Debug, Clone)]
pub enum RequestUsageScope {
    Users,
    Keys { user_id: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestUsage {
    pub id: String,
    pub current_concurrency: Option<u64>,
    pub current_rpm: Option<u64>,
}
