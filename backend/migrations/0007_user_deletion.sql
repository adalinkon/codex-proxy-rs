-- 删除用户后保留身份与费用外键，防止同名重建重置历史额度或接管请求记录。
alter table admin_users
  add column deleted_at timestamptz,
  add constraint deleted_user_disabled_ck check (deleted_at is null or not enabled);
