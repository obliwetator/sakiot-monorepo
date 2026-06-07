use opentelemetry_sdk::Resource;
use std::env;
use std::io::Write;
use tracing_subscriber::{layer::SubscriberExt, Layer, Registry};

pub const SERVICE_NAME: &str = "web_server";

fn warn_startup(msg: &str) {
    #[allow(clippy::print_stderr)]
    {
        let _ = writeln!(std::io::stderr(), "Warning: {msg}");
    }
}

fn service_instance_id(port: u16) -> String {
    env::var("OTEL_SERVICE_INSTANCE_ID")
        .or_else(|_| env::var("SERVICE_INSTANCE_ID"))
        .unwrap_or_else(|_| {
            let host = env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string());
            format!("{host}:{port}:{}", std::process::id())
        })
}

pub fn init_telemetry(port: u16) {
    let otlp_exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            warn_startup(&format!(
                "failed to create OTLP span exporter, traces disabled: {e}"
            ));
            let fmt_layer = tracing_subscriber::fmt::layer()
                .pretty()
                .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
            let subscriber = Registry::default().with(fmt_layer);
            let _ = tracing::subscriber::set_global_default(subscriber);
            return;
        }
    };

    let metrics_exporter = match opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
    {
        Ok(e) => e,
        Err(e) => {
            warn_startup(&format!(
                "failed to create OTLP metric exporter, metrics disabled: {e}"
            ));
            let fmt_layer = tracing_subscriber::fmt::layer()
                .pretty()
                .with_filter(tracing_subscriber::filter::LevelFilter::INFO);
            let subscriber = Registry::default().with(fmt_layer);
            let _ = tracing::subscriber::set_global_default(subscriber);
            return;
        }
    };

    let resource = Resource::builder_empty()
        .with_attributes([
            opentelemetry::KeyValue::new("service.name", SERVICE_NAME),
            opentelemetry::KeyValue::new("service.instance.id", service_instance_id(port)),
            opentelemetry::KeyValue::new("service.port", i64::from(port)),
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

    let tracer = opentelemetry::global::tracer(SERVICE_NAME);
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    let fmt_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_filter(tracing_subscriber::filter::LevelFilter::INFO);

    let subscriber = Registry::default().with(telemetry).with(fmt_layer);

    if let Err(e) = tracing::subscriber::set_global_default(subscriber) {
        warn_startup(&format!("failed to set global tracing subscriber: {e}"));
    }
}
