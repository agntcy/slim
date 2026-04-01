import os

import uvicorn
from google.adk.a2a.utils.agent_to_a2a import to_a2a

from k8s_troubleshooting_agent.agent import root_agent

HOST = os.getenv("HOST", "localhost")
PORT = int(os.getenv("PORT", "8000"))

app = to_a2a(root_agent, host=HOST, port=PORT)

if __name__ == "__main__":
    uvicorn.run(app, host=HOST, port=PORT)
