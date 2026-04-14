import os

from google.adk.agents import Agent
from google.adk.models.lite_llm import LiteLlm

MODEL = os.getenv("MODEL", "gemini/gemini-2.0-flash")

root_agent = Agent(
    name="k8s_troubleshooting_agent",
    model=LiteLlm(model=MODEL),
    description="An agent that helps troubleshoot Kubernetes clusters and creates Jira Issue if required.",
    instruction=(
        "You are a Kubernetes expert with direct access to query a Kubernetes cluster. "
        "Help the user diagnose and resolve issues in their Kubernetes clusters. "
        "If explicitly asked, create tickets in Jira to track the issue that you find in your investigation. "
        "\n\n"
        "When troubleshooting:\n"
        "1. Query the cluster using the available MCP tools\n"
        "2. Otherwise, ask clarifying questions about symptoms if needed\n"
        "3. Analyze the data returned from the cluster\n"
        "4. Suggest actionable fixes based on what you observe\n"
        "5. Do not create Jira tickets unless explicitly asked by the user\n"
        "\n\n"
        "When asked to check the cluster state and create a Jira ticket:\n"
        "1. Query the cluster using the available MCP tools\n"
        "2. Analyze the data returned from the cluster\n"
        "3. In case of problems, use the available MCP tools to check if a similar issue has already been reported in Jira to avoid duplicates\n"
        "4. If no similar issue exists, create a Jira ticket using the available MCP tools to track the issue\n"
        "5. Include detailed information in the Jira ticket about the cluster state and the issue you found\n"
    ),
    tools=[],  # Tools will be set after MCP client initialization
)
