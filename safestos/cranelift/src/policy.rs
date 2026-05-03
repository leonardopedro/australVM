use cedar_policy::{Authorizer, Context, Entities, PolicySet, Request, Response, EntityUid};
use std::str::FromStr;

thread_local! {
    pub static CEDAR_ENGINE: std::cell::RefCell<CedarVmEngine> = std::cell::RefCell::new(CedarVmEngine::new());
}

pub struct CedarVmEngine {
    authorizer: Authorizer,
    policies: PolicySet,
    entities: Entities,
}

impl CedarVmEngine {
    pub fn new() -> Self {
        // Initialize an empty policy set and entities
        let policies = PolicySet::new();
        let entities = Entities::empty();
        Self {
            authorizer: Authorizer::new(),
            policies,
            entities,
        }
    }

    pub fn load_policy(&mut self, policy_src: &str) -> Result<(), String> {
        let new_policies = PolicySet::from_str(policy_src).map_err(|e| format!("Policy parse error: {}", e))?;
        self.policies = new_policies;
        Ok(())
    }

    pub fn is_authorized(&self, principal_mod: &str, action: &str, resource_mod: &str) -> Result<bool, String> {
        let principal = EntityUid::from_str(&format!("Module::\"{}\"", principal_mod))
            .map_err(|e| format!("Invalid principal: {}", e))?;
        let action = EntityUid::from_str(&format!("Action::\"{}\"", action))
            .map_err(|e| format!("Invalid action: {}", e))?;
        let resource = EntityUid::from_str(&format!("Module::\"{}\"", resource_mod))
            .map_err(|e| format!("Invalid resource: {}", e))?;

        let request = Request::new(
            principal,
            action,
            resource,
            Context::empty(),
            None,
        ).map_err(|e| format!("Request creation failed: {}", e))?;

        let response: Response = self.authorizer.is_authorized(&request, &self.policies, &self.entities);

        match response.decision() {
            cedar_policy::Decision::Allow => Ok(true),
            cedar_policy::Decision::Deny => Ok(false),
        }
    }
}
