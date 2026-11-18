// allow.cedar — the permissive half of the documented deny/allow pair.
//
// Used with caps-exec.json (or any cell's own table) it permits every
// request, so it is the control run: if a build or load still fails with
// this policy, the cause is not the gate.
permit(principal, action, resource);
