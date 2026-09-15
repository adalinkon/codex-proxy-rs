-- 将旧的全量授权固化为当前分组；新增分组须显式授权，未分组账号不再可用。
insert into user_account_groups (user_id, account_group_id)
select u.id, g.id
from admin_users u cross join account_groups g
where u.all_groups and u.deleted_at is null
on conflict do nothing;

alter table admin_users drop constraint regular_user_groups_ck;
alter table admin_users drop column all_groups;
