use opentelemetry_sdk::Resource;
use std::error::Error;
use std::io::IsTerminal;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{
    Layer, Registry,
    filter::{EnvFilter, FilterExt, LevelFilter, Targets},
    layer::SubscriberExt,
    registry::LookupSpan,
};

/// Days of JSON log files kept in `logs/`; older files are deleted on rotation.
const LOG_FILE_DAYS: usize = 14;

const SUPPRESSED_SONGBIRD_UDP_RX_LOGS: [&str; 2] = [
    "songbird::driver::tasks::udp_rx=off",
    "songbird::driver::tasks::udp_rx::ssrc_state=off",
];

pub fn init_telemetry() -> Result<(), Box<dyn Error + Send + Sync>> {
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()?;

    let metrics_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()?;

    let instance_id = std::env::var("BOT_INSTANCE_ID")
        .unwrap_or_else(|_| format!("{}-{}", crate::config::SERVICE_NAME, std::process::id()));
    let resource = Resource::builder_empty()
        .with_attributes([
            opentelemetry::KeyValue::new("service.name", crate::config::SERVICE_NAME),
            opentelemetry::KeyValue::new("service.instance.id", instance_id),
        ])
        .build();

    let tracer_provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(otlp_exporter)
        .with_resource(resource.clone())
        .build();

    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(opentelemetry_sdk::metrics::PeriodicReader::builder(metrics_exporter).build())
        .with_resource(resource)
        .build();

    opentelemetry::global::set_tracer_provider(tracer_provider);
    opentelemetry::global::set_meter_provider(meter_provider);

    let tracer = opentelemetry::global::tracer(crate::config::SERVICE_NAME);
    let log_filter = log_filter()?;

    // Traces carry only this crate's spans. serenity's and songbird's gateway
    // heartbeat and receive spans were a dozen a second and all of the volume.
    let telemetry = tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_filter(
            log_filter
                .clone()
                .and(Targets::new().with_target("fbi_agent", LevelFilter::TRACE)),
        );

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("fbi-agent.log")
        .max_log_files(LOG_FILE_DAYS)
        .build("logs")?;
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // We intentionally leak the guard so the background writer stays alive.
    // Ideally this would be returned and kept in main(), but leaking it works
    // for global long-running daemons.
    std::mem::forget(_guard);

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(non_blocking)
        .with_span_events(FmtSpan::NONE)
        .with_filter(log_filter.clone());

    let subscriber = Registry::default()
        .with(telemetry)
        .with(file_layer)
        .with(console_layer().with_filter(log_filter));

    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

/// Colored multi-line output in a terminal; one plain line per event under
/// systemd, so the journal and Loki hold whole events without color escapes.
fn console_layer<S>() -> Box<dyn Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    if std::io::stdout().is_terminal() {
        tracing_subscriber::fmt::layer()
            .pretty()
            .with_span_events(FmtSpan::NONE)
            .boxed()
    } else {
        tracing_subscriber::fmt::layer()
            .compact()
            .with_ansi(false)
            .with_span_events(FmtSpan::NONE)
            .boxed()
    }
}

fn log_filter() -> Result<EnvFilter, Box<dyn Error + Send + Sync>> {
    let mut filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    for directive in SUPPRESSED_SONGBIRD_UDP_RX_LOGS {
        filter = filter.add_directive(directive.parse()?);
    }

    Ok(filter)
}
