-- 保留已有管理员标识及密码；同表扩展角色，避免迁移既有审计外键。
alter table admin_users
  add column role text not null default 'admin' check (role in ('admin', 'user')),
  add column enabled boolean not null default true,
  add column auth_revision bigint not null default 0 check (auth_revision >= 0),
  add column all_groups boolean not null default true,
  add column daily_limit_usd numeric(20,10) not null default 0 check (daily_limit_usd >= 0),
  add column weekly_limit_usd numeric(20,10) not null default 0 check (weekly_limit_usd >= 0),
  add column max_concurrency bigint not null default 0 check (max_concurrency >= 0),
  add column requests_per_minute bigint not null default 0 check (requests_per_minute >= 0),
  add constraint regular_user_groups_ck check (role = 'admin' or not all_groups);

create table user_account_groups (
  user_id text not null references admin_users(id) on delete restrict,
  account_group_id text not null references account_groups(id) on delete restrict,
  primary key (user_id, account_group_id)
);

alter table client_api_keys add column user_id text references admin_users(id) on delete restrict;
update client_api_keys set user_id = (select id from admin_users order by created_at, id limit 1);
alter table client_api_keys alter column user_id set not null;
create index client_api_keys_user_idx on client_api_keys(user_id);

create table user_budget_windows (
  user_id text primary key references admin_users(id) on delete restrict,
  daily_start timestamptz not null,
  daily_end timestamptz not null,
  weekly_start timestamptz not null,
  weekly_end timestamptz not null,
  daily_used_usd numeric(20,10) not null default 0 check (daily_used_usd >= 0),
  weekly_used_usd numeric(20,10) not null default 0 check (weekly_used_usd >= 0)
);

-- 用户账本不引用 Key，删除 Key 或清理请求日志不会删除费用与幂等标记。
create table user_charge_events (
  request_id text primary key,
  user_id text not null references admin_users(id) on delete restrict,
  client_api_key_ref text not null,
  amount_usd numeric(20,10) not null check (amount_usd >= 0),
  completed_at timestamptz not null
);
create index user_charge_events_user_idx on user_charge_events(user_id, completed_at);
insert into user_charge_events
select e.request_id, k.user_id, e.client_api_key_id, e.amount_usd, e.completed_at
from client_key_charge_events e join client_api_keys k on k.id = e.client_api_key_id;

-- 旧 Key 各自窗口保持原样；管理员用户窗口从最早仍有效的七天窗口建立。
insert into user_budget_windows
with anchors as (
  select k.user_id, min(w.weekly_start) as week
  from client_key_budget_windows w join client_api_keys k on k.id = w.client_api_key_id
  where w.weekly_end > now()
  group by k.user_id
), today as (
  select date_trunc('day', now() at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as day
)
select a.user_id, t.day, t.day + interval '24 hours', a.week, a.week + interval '168 hours',
       coalesce(sum(e.amount_usd) filter (where e.completed_at >= t.day and e.completed_at < t.day + interval '24 hours'), 0),
       coalesce(sum(e.amount_usd) filter (where e.completed_at >= a.week and e.completed_at < a.week + interval '168 hours'), 0)
from anchors a cross join today t left join user_charge_events e on e.user_id = a.user_id
group by a.user_id, t.day, a.week;

alter table model_requests add column user_id text references admin_users(id) on delete restrict;
update model_requests set user_id = (select id from admin_users order by created_at, id limit 1);
create index model_requests_user_started_idx on model_requests(user_id, started_at desc, id desc);
