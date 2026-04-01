from google.adk.agents import Agent

root_agent = Agent(
    name="k8s_troubleshooting_agent",
    model="gemini-2.0-flash",
    description="An agent that helps troubleshoot Kubernetes clusters.",
    instruction=(
        "You are a Kubernetes expert. Help the user diagnose and resolve issues "
        "in their Kubernetes clusters. Ask clarifying questions about symptoms, "
        "inspect provided manifests or error messages, and suggest actionable fixes."
    ),
)
