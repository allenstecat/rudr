use k8s_openapi::api::batch::v1 as batchapi;
use k8s_openapi::api::core::v1 as api;
use k8s_openapi::apimachinery::pkg::apis::meta::v1 as meta;
use kube::api::PostParams;
use kube::client::APIClient;
use std::collections::BTreeMap;

use crate::schematic::component::Component;
use crate::workload_type::{InstigatorResult, ParamMap};

/// WorkloadMetadata contains common data about a workload.
///
/// Individual workload types can embed this field.
pub struct WorkloadMetadata {
    /// Name is the name of the release
    pub name: String,
    /// Component name is the name of this particular workload component
    pub component_name: String,
    /// Instance name is the name of this component's instance (unique name)
    pub instance_name: String,
    /// Namespace is the Kubernetes namespace into which this component should
    /// be placed.
    pub namespace: String,
    /// Definition is the definition of the component.
    pub definition: Component,
    /// Client is the Kubernetes API client
    pub client: APIClient,
    /// Params contains a map of parameters that were supplied for this workload
    pub params: ParamMap,
    /// Owner Ref is the Kubernetes owner reference
    ///
    /// This tells Kubenretes what object "owns" this workload and is responsible
    /// for cleaning it up.
    pub owner_ref: Option<Vec<meta::OwnerReference>>,
}

type Labels = BTreeMap<String, String>;

/// JobBuilder builds new jobs specific to Scylla
///
/// This hides many of the details of building a Job, exposing only
/// parameters common to Scylla workload types.
pub(crate) struct JobBuilder {
    component: Component,
    labels: Labels,
    name: String,
    restart_policy: String,
    owner_ref: Option<Vec<meta::OwnerReference>>,
    parallelism: Option<i32>,
}

impl JobBuilder {
    /// Create a JobBuilder
    pub fn new(instance_name: String, component: Component) -> Self {
        JobBuilder {
            name: instance_name,
            component: component,
            labels: BTreeMap::new(),
            restart_policy: "Never".to_string(),
            owner_ref: None,
            parallelism: None,
        }
    }
    /// Add labels
    pub fn labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }
    /// Set the restart policy
    pub fn restart_policy(mut self, policy: String) -> Self {
        self.restart_policy = policy;
        self
    }
    /// Set the owner refence for the job and the pod
    pub fn owner_ref(mut self, owner: Option<Vec<meta::OwnerReference>>) -> Self {
        self.owner_ref = owner;
        self
    }
    /// Set the parallelism
    pub fn parallelism(mut self, count: i32) -> Self {
        self.parallelism = Some(count);
        self
    }
    pub fn to_job(self) -> batchapi::Job {
        batchapi::Job {
            // TODO: Could make this generic.
            metadata: Some(meta::ObjectMeta {
                name: Some(self.name.clone()),
                labels: Some(self.labels.clone()),
                owner_references: self.owner_ref.clone(),
                ..Default::default()
            }),
            spec: Some(batchapi::JobSpec {
                backoff_limit: Some(4),
                parallelism: self.parallelism,
                template: api::PodTemplateSpec {
                    metadata: Some(meta::ObjectMeta {
                        name: Some(self.name.clone()),
                        labels: Some(self.labels.clone()),
                        owner_references: self.owner_ref.clone(),
                        ..Default::default()
                    }),
                    spec: Some(
                        self.component
                            .to_pod_spec_with_policy(self.restart_policy.clone()),
                    ),
                },
                ..Default::default()
            }),
            ..Default::default()
        }
    }
    pub fn do_request(self, client: APIClient, namespace: String) -> InstigatorResult {
        let job = self.to_job();
        let pp = kube::api::PostParams::default();
        // Right now, the Batch API is not transparent through Kube.
        // Next release of Kube will fix this
        let batch = kube::api::RawApi {
            group: "batch".into(),
            resource: "jobs".into(),
            prefix: "apis".into(),
            namespace: Some(namespace),
            version: "v1".into(),
        };

        let req = batch.create(&pp, serde_json::to_vec(&job)?)?;
        client.request::<batchapi::Job>(req)?;
        Ok(())
    }
}

pub struct ServiceBuilder {
    component: Component,
    labels: Labels,
    name: String,
    owner_ref: Option<Vec<meta::OwnerReference>>,
}

impl ServiceBuilder {
    pub fn new(instance_name: String, component: Component) -> Self {
        ServiceBuilder {
            name: instance_name,
            component: component,
            labels: Labels::new(),
            owner_ref: None,
        }
    }
    pub fn labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }
    pub fn owner_reference(mut self, owner_ref: Option<Vec<meta::OwnerReference>>) -> Self {
        self.owner_ref = owner_ref;
        self
    }
    pub fn to_service(self) -> Option<api::Service> {
        self.component.clone().listening_port().and_then(|port| {
            Some(api::Service {
                metadata: Some(meta::ObjectMeta {
                    name: Some(self.name.clone()),
                    labels: Some(self.labels.clone()),
                    owner_references: self.owner_ref.clone(),
                    ..Default::default()
                }),
                spec: Some(api::ServiceSpec {
                    selector: Some(self.labels),
                    ports: Some(vec![port.to_service_port()]),
                    ..Default::default()
                }),
                ..Default::default()
            })
        })
    }
    pub fn do_request(self, client: APIClient, namespace: String) -> InstigatorResult {
        match self.to_service() {
            Some(svc) => {
                info!("Service:\n{}", serde_json::to_string_pretty(&svc).unwrap());
                let pp = PostParams::default();
                kube::api::Api::v1Service(client)
                    .within(namespace.as_str())
                    .create(&pp, serde_json::to_vec(&svc)?)?;
                Ok(())
            }
            // No service to create
            None => {
                info!("Not attaching service to pod with no container ports.");
                Ok(())
            }
        }
    }
}
