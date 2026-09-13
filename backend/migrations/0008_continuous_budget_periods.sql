-- 固定北京时间周期同时供查询、准入和结算使用，不依赖是否发生请求。
create function budget_periods(anchor timestamptz, at_time timestamptz)
returns table(daily_start timestamptz, daily_end timestamptz, weekly_start timestamptz, weekly_end timestamptz)
language sql immutable strict parallel safe as $$
  select day, day + interval '24 hours', week, week + interval '168 hours'
  from (
    select date_bin(interval '24 hours', at_time, origin) as day,
           date_bin(interval '168 hours', at_time, origin) as week
    from (select date_trunc('day', anchor at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as origin) a
  ) p
$$;

-- 迁移只能根据完整账本重建；累计值与账本不一致时拒绝升级，不能静默抹去用量。
do $$
begin
  if exists (
    select 1 from client_key_budget_windows w
    cross join lateral (
      select coalesce(sum(amount_usd) filter (where completed_at >= w.daily_start and completed_at < w.daily_end), 0) as day,
             coalesce(sum(amount_usd) filter (where completed_at >= w.weekly_start and completed_at < w.weekly_end), 0) as week
      from client_key_charge_events where client_api_key_id = w.client_api_key_id
    ) e where e.day <> w.daily_used_usd or e.week <> w.weekly_used_usd
  ) or exists (
    select 1 from user_budget_windows w
    cross join lateral (
      select coalesce(sum(amount_usd) filter (where completed_at >= w.daily_start and completed_at < w.daily_end), 0) as day,
             coalesce(sum(amount_usd) filter (where completed_at >= w.weekly_start and completed_at < w.weekly_end), 0) as week
      from user_charge_events where user_id = w.user_id
    ) e where e.day <> w.daily_used_usd or e.week <> w.weekly_used_usd
  ) then
    raise exception 'budget ledger does not match stored usage; restore complete charge events before upgrading';
  end if;
end
$$;

alter table admin_users add column budget_reset_at timestamptz;

-- 仅保存重置请求的去重结果，不是操作审计；旧操作重试不能再次清零。
create table user_budget_reset_operations (
  operation_id uuid primary key,
  user_id text not null references admin_users(id) on delete restrict,
  reset_at timestamptz not null
);

insert into user_budget_windows(user_id,daily_start,daily_end,weekly_start,weekly_end,daily_used_usd,weekly_used_usd)
select u.id,p.daily_start,p.daily_end,p.weekly_start,p.weekly_end,
       coalesce(sum(e.amount_usd) filter (where e.completed_at >= p.daily_start and e.completed_at < p.daily_end),0),
       coalesce(sum(e.amount_usd) filter (where e.completed_at >= p.weekly_start and e.completed_at < p.weekly_end),0)
from admin_users u cross join lateral budget_periods(u.created_at,now()) p
left join user_charge_events e on e.user_id=u.id
group by u.id,p.daily_start,p.daily_end,p.weekly_start,p.weekly_end
on conflict(user_id) do update set daily_start=excluded.daily_start,daily_end=excluded.daily_end,
  weekly_start=excluded.weekly_start,weekly_end=excluded.weekly_end,
  daily_used_usd=excluded.daily_used_usd,weekly_used_usd=excluded.weekly_used_usd;

insert into client_key_budget_windows(client_api_key_id,daily_start,daily_end,weekly_start,weekly_end,daily_used_usd,weekly_used_usd)
select k.id,p.daily_start,p.daily_end,p.weekly_start,p.weekly_end,
       coalesce(sum(e.amount_usd) filter (where e.completed_at >= p.daily_start and e.completed_at < p.daily_end),0),
       coalesce(sum(e.amount_usd) filter (where e.completed_at >= p.weekly_start and e.completed_at < p.weekly_end),0)
from client_api_keys k cross join lateral budget_periods(k.created_at,now()) p
left join client_key_charge_events e on e.client_api_key_id=k.id
group by k.id,p.daily_start,p.daily_end,p.weekly_start,p.weekly_end
on conflict(client_api_key_id) do update set daily_start=excluded.daily_start,daily_end=excluded.daily_end,
  weekly_start=excluded.weekly_start,weekly_end=excluded.weekly_end,
  daily_used_usd=excluded.daily_used_usd,weekly_used_usd=excluded.weekly_used_usd;

-- 只投影当前周期的计数，不扫描费用事件；未请求的身份也有明确的下次重置时间。
create view user_budget_status as
select u.id as user_id,p.*,
  case when w.daily_start=p.daily_start and w.daily_end=p.daily_end then w.daily_used_usd else 0 end as daily_used_usd,
  case when w.weekly_start=p.weekly_start and w.weekly_end=p.weekly_end then w.weekly_used_usd else 0 end as weekly_used_usd
from admin_users u cross join lateral budget_periods(coalesce(u.budget_reset_at,u.created_at),now()) p
left join user_budget_windows w on w.user_id=u.id;

create view client_key_budget_status as
select k.id as client_api_key_id,p.*,
  case when w.daily_start=p.daily_start and w.daily_end=p.daily_end then w.daily_used_usd else 0 end as daily_used_usd,
  case when w.weekly_start=p.weekly_start and w.weekly_end=p.weekly_end then w.weekly_used_usd else 0 end as weekly_used_usd
from client_api_keys k cross join lateral budget_periods(k.created_at,now()) p
left join client_key_budget_windows w on w.client_api_key_id=k.id;
