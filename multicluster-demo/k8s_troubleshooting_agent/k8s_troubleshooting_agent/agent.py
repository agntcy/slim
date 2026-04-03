import os

from google.adk.agents import Agent
from google.adk.models.lite_llm import LiteLlm

from k8s_troubleshooting_agent.tools import (
    call_mcp_tool,
    list_available_mcp_tools,
)

MODEL = os.getenv("MODEL", "gemini/gemini-2.0-flash")

root_agent = Agent(
    name="k8s_troubleshooting_agent",
    model=LiteLlm(model=MODEL),
    description="An agent that helps troubleshoot Kubernetes clusters.",
    instruction=(
        "You are a Kubernetes expert with direct access to query a Kubernetes cluster. "
        "Help the user diagnose and resolve issues in their Kubernetes clusters. "
        "\n\n"
        "When troubleshooting:\n"
        "1. First, use list_available_mcp_tools to discover what tools are available\n"
        "2. Use call_mcp_tool to query the cluster\n"
        "3. Ask clarifying questions about symptoms if needed\n"
        "4. Analyze the data returned from the cluster\n"
        "5. Suggest actionable fixes based on what you observe\n"
        "\n"
        "Always check the actual cluster state before making recommendations."
    ),
    tools=[
        list_available_mcp_tools,
        call_mcp_tool,
    ],
)
