alter table public.bot_instances add column if not exists grpc_address text;

comment on column public.bot_instances.grpc_address is
    'host:port this instance serves gRPC on, so the web server can route per-guild requests to the instance that owns the voice lease. NULL for instances predating this column.';
