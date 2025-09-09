# Travel Planner Example
This is the Travel Planner Example from the A2A repo, modified to use SLIM A2A

## Getting started

1. Create an environment file with your API key:
> You need to set the value corresponding AZURE_OPENAI_API_KEY.

   ```bash
   echo "AZURE_OPENAI_API_KEY=your_api_key_here" > .env
   ```

2. Start SLIM
  ```bash
  cd ../../../testing/
  task run:slim
  ```

3. Start the server
    ```bash
    cd -
    uv run examples/travel_planner_agent/server.py
    ```

4. Run the loop client
    ```bash
    uv run examples/travel_planner_agent/client.py
    ```