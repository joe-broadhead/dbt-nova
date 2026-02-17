-- Retention report step 1
select date_trunc('week', current_timestamp()) as week_start, count(*) as active_users;
