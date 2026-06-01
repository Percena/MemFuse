use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RetrievalStep {
    pub stage: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RetrievalTrajectory {
    pub steps: Vec<RetrievalStep>,
}

impl RetrievalTrajectory {
    pub fn record(&mut self, stage: &str, detail: &str) {
        self.steps.push(RetrievalStep {
            stage: stage.to_owned(),
            detail: detail.to_owned(),
        });
    }
}
