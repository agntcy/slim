import json
import os
import pathlib
import sys
from collections.abc import AsyncGenerator
from typing import Any

from langchain_core.messages import HumanMessage, SystemMessage
from langchain_openai import AzureChatOpenAI


class TravelPlannerAgent:
    """travel planner Agent."""

    def __init__(self) -> None:
        """Initialize the travel dialogue model."""
        try:
            config_path = pathlib.Path(__file__).parent / "config.json"
            with config_path.open() as f:
                config = json.load(f)
            if not os.getenv(config["api_key"]):
                print(f"{config['api_key']} environment variable not set.")
                sys.exit(1)
            api_key = os.getenv(config["api_key"])

            self.model = AzureChatOpenAI(
                model=config["model_name"],
                azure_endpoint=config["azure_endpoint"],
                api_key=api_key,  # type: ignore
                temperature=config[
                    "temperature"
                ],  # Control the generation randomness (0-2, higher values indicate greater randomness)
                api_version=config["api_version"],
            )
        except FileNotFoundError:
            print("Error: The configuration file config.json cannot be found.")
            sys.exit()
        except KeyError as e:
            print(f"The configuration file is missing required fields: {e}")
            sys.exit()

    async def stream(self, query: str) -> AsyncGenerator[dict[str, Any], None]:
        """Stream the response of the large model back to the client."""
        try:
            # Initialize the conversation history (system messages can be added)
            messages: list[HumanMessage | SystemMessage] = [
                SystemMessage(
                    content="""
                You are an expert travel assistant specializing in trip planning, destination information,
                and travel recommendations. Your goal is to help users plan enjoyable, safe, and
                realistic trips based on their preferences and constraints.

                When providing information:
                - Be specific and practical with your advice
                - Consider seasonality, budget constraints, and travel logistics
                - Highlight cultural experiences and authentic local activities
                - Include practical travel tips relevant to the destination
                - Format information clearly with headings and bullet points when appropriate

                For itineraries:
                - Create realistic day-by-day plans that account for travel time between attractions
                - Balance popular tourist sites with off-the-beaten-path experiences
                - Include approximate timing and practical logistics
                - Suggest meal options highlighting local cuisine
                - Consider weather, local events, and opening hours in your planning

                Always maintain a helpful, enthusiastic but realistic tone and acknowledge
                any limitations in your knowledge when appropriate.
                """
                )
            ]

            # Add the user message to the history.
            messages.append(HumanMessage(content=query))

            # Invoke the model in streaming mode to generate a response.
            async for chunk in self.model.astream(messages):
                # Return the text content block.
                if hasattr(chunk, "content") and chunk.content:
                    yield {"content": chunk.content, "done": False}
            yield {"content": "", "done": True}

        except Exception as e:
            print(f"error：{e!s}")
            yield {
                "content": "Sorry, an error occurred while processing your request.",
                "done": True,
            }
