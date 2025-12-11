#!/usr/bin/env node

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { ChromaClient, TransformersEmbeddingFunction } from "chromadb";

const CHROMA_HOST = process.env.CHROMA_HOST || "localhost";
const CHROMA_PORT = process.env.CHROMA_PORT || "8000";
const COLLECTION_NAME = process.env.COLLECTION_NAME || "codebase";
const GIT_BRANCH = process.env.GIT_BRANCH || "";

class VectorMCPServer {
  constructor() {
    this.server = new Server(
      {
        name: "vector-mcp-server",
        version: "2.0.0",
      },
      {
        capabilities: {
          tools: {},
        },
      }
    );

    this.chromaClient = new ChromaClient({
      path: `http://${CHROMA_HOST}:${CHROMA_PORT}`,
    });

    // Initialize embedding function - same model used during indexing
    this.embeddingFunction = new TransformersEmbeddingFunction({
      model: "Xenova/all-MiniLM-L6-v2",
    });

    this.setupToolHandlers();
    this.setupErrorHandling();
  }

  setupErrorHandling() {
    this.server.onerror = (error) => {
      console.error("[MCP Error]", error);
    };

    process.on("SIGINT", async () => {
      await this.server.close();
      process.exit(0);
    });
  }

  setupToolHandlers() {
    this.server.setRequestHandler(ListToolsRequestSchema, async () => ({
      tools: [
        {
          name: "query",
          description:
            "🔍 PRIMARY TOOL: Semantic search across the entire codebase. Use this for ANY code search question. Claude should craft the query to find what's needed. Examples: 'authentication logic', 'payment processing', 'test setup patterns', 'error handling in services', 'React components that use hooks'. Returns complete code chunks (2000+ chars each) - NO FILE READS NEEDED after this.",
          inputSchema: {
            type: "object",
            properties: {
              query: {
                type: "string",
                description: "Natural language query describing what code to find. Be specific and include context.",
              },
              n_results: {
                type: "number",
                description: "Number of code chunks to return (default: 10, max: 50)",
                default: 10,
              },
              filter_path: {
                type: "string",
                description: "Optional path filter to limit search scope (e.g., 'App/Services/', 'App/Screens/Auth/')",
              },
            },
            required: ["query"],
          },
        },
        {
          name: "query_similar",
          description:
            "🔗 Find code similar to a reference file. Uses the reference file's content as a semantic query to find related implementations, patterns, or dependencies. Perfect for 'show me similar code', 'what else works like this?', 'find related files'.",
          inputSchema: {
            type: "object",
            properties: {
              reference_path: {
                type: "string",
                description: "File path to use as reference for similarity search",
              },
              n_results: {
                type: "number",
                description: "Number of similar code chunks to return (default: 8)",
                default: 8,
              },
            },
            required: ["reference_path"],
          },
        },
        {
          name: "trace_path",
          description:
            "🛤️ Trace execution paths and code flows. Use this to answer 'how do I navigate to X screen?', 'what happens when user clicks Y?', 'trace the flow from A to B'. Returns the sequence of files/functions involved in a code path with complete implementations.",
          inputSchema: {
            type: "object",
            properties: {
              start_point: {
                type: "string",
                description: "Starting point (e.g., 'Login button click', 'app launch', 'deeplink handling', 'HomeScreen')",
              },
              end_point: {
                type: "string",
                description: "Optional end point (e.g., 'Dashboard screen', 'API call', 'user authenticated')",
              },
              include_depth: {
                type: "number",
                description: "How many levels deep to trace (default: 15)",
                default: 15,
              },
            },
            required: ["start_point"],
          },
        },
        {
          name: "find_reproduction",
          description:
            "🐛 Find code paths to reproduce issues or trigger features. Use for 'how to reproduce X bug?', 'how to trigger Y feature?', 'steps to reach Z state?'. Returns entry points, navigation steps, and relevant code with instructions.",
          inputSchema: {
            type: "object",
            properties: {
              target: {
                type: "string",
                description: "What to reproduce (e.g., 'checkout error', 'payment flow', 'notification display', 'crash on profile')",
              },
              context: {
                type: "string",
                description: "Optional context (e.g., 'on Android', 'with expired token', 'as guest user')",
              },
            },
            required: ["target"],
          },
        },
        {
          name: "map_dependencies",
          description:
            "📊 Map file dependencies and call relationships. Use for 'what does X import?', 'what files depend on Y?', 'show me the dependency graph for Z'. Returns imports, exports, and usage relationships with code examples.",
          inputSchema: {
            type: "object",
            properties: {
              file_path: {
                type: "string",
                description: "File to analyze dependencies for",
              },
              direction: {
                type: "string",
                description: "Direction: 'imports' (what this file uses), 'imported_by' (what uses this file), or 'both' (default: 'both')",
                default: "both",
              },
              depth: {
                type: "number",
                description: "How many levels deep to traverse (default: 2)",
                default: 2,
              },
            },
            required: ["file_path"],
          },
        },
        {
          name: "stats",
          description:
            "📊 Get vector store statistics (document count, git hash, index freshness)",
          inputSchema: {
            type: "object",
            properties: {},
          },
        },
      ],
    }));

    this.server.setRequestHandler(CallToolRequestSchema, async (request) => {
      const { name, arguments: args } = request.params;

      try {
        switch (name) {
          case "query":
            return await this.query(args);
          case "query_similar":
            return await this.querySimilar(args);
          case "trace_path":
            return await this.tracePath(args);
          case "find_reproduction":
            return await this.findReproduction(args);
          case "map_dependencies":
            return await this.mapDependencies(args);
          case "stats":
            return await this.getStats();
          default:
            throw new Error(`Unknown tool: ${name}`);
        }
      } catch (error) {
        return {
          content: [
            {
              type: "text",
              text: `Error: ${error.message}`,
            },
          ],
          isError: true,
        };
      }
    });
  }

  // ============================================================================
  // HELPER: BUILD WHERE CLAUSE WITH BRANCH FILTER
  // ============================================================================

  buildWhereClause(additionalFilters = {}) {
    const where = { ...additionalFilters };

    // Always filter by current branch if available
    if (GIT_BRANCH) {
      where.git_branch = GIT_BRANCH;
    }

    return Object.keys(where).length > 0 ? where : undefined;
  }

  // ============================================================================
  // PRIMARY SEARCH METHOD
  // ============================================================================

  async query(args) {
    const { query, n_results = 10, filter_path } = args;

    try {
      const collection = await this.chromaClient.getCollection({
        name: COLLECTION_NAME,
        embeddingFunction: this.embeddingFunction,
      });

      const pathFilter = filter_path
        ? { file_path: { $contains: filter_path } }
        : {};

      const where = this.buildWhereClause(pathFilter);

      const results = await collection.query({
        queryTexts: [query],
        nResults: Math.min(n_results, 50),
        where: where,
      });

      if (!results.documents[0] || results.documents[0].length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `No results found for query: "${query}"${filter_path ? ` in path: ${filter_path}` : ''}`,
            },
          ],
        };
      }

      const formattedResults = results.documents[0]
        .map((doc, idx) => {
          const metadata = results.metadatas[0][idx];
          const distance = results.distances[0][idx];
          const similarity = (1 - distance).toFixed(3);

          return `## Result ${idx + 1} (similarity: ${similarity})
**File**: ${metadata.file_path}:${metadata.start_line || "1"}
**Type**: ${metadata.file_type || "unknown"}

\`\`\`${metadata.language || ""}
${doc}
\`\`\`
`;
        })
        .join("\n\n---\n\n");

      return {
        content: [
          {
            type: "text",
            text: `# 🔍 Query Results: "${query}"

**Results**: ${results.documents[0].length} code chunks found
${filter_path ? `**Scope**: ${filter_path}` : '**Scope**: Entire codebase'}

**🤖 TO THE AI**: Complete code chunks (2000+ chars each) are provided below. NO FILE READS NEEDED. Analyze and answer the user's question directly using this code.

---

${formattedResults}

---

📌 All relevant code provided above. Use this to answer the user's question.`,
          },
        ],
      };
    } catch (error) {
      if (error.message.includes("does not exist")) {
        return {
          content: [
            {
              type: "text",
              text: `Collection "${COLLECTION_NAME}" does not exist. Run: docker compose up -d`,
            },
          ],
        };
      }
      throw error;
    }
  }

  // ============================================================================
  // SIMILARITY SEARCH
  // ============================================================================

  async querySimilar(args) {
    const { reference_path, n_results = 8 } = args;

    try {
      const collection = await this.chromaClient.getCollection({
        name: COLLECTION_NAME,
        embeddingFunction: this.embeddingFunction,
      });

      // Get the reference file's content
      const where = this.buildWhereClause({ file_path: { $contains: reference_path } });
      const refResults = await collection.get({
        where: where,
        limit: 1,
        include: ["documents", "metadatas"],
      });

      if (!refResults.documents || refResults.documents.length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `Reference file not found: ${reference_path}`,
            },
          ],
        };
      }

      // Use content as query to find similar
      const similarWhere = this.buildWhereClause();
      const results = await collection.query({
        queryTexts: [refResults.documents[0]],
        nResults: n_results + 1, // +1 to exclude self
        where: similarWhere,
      });

      // Filter out reference file
      const filtered = {
        documents: [[]],
        metadatas: [[]],
        distances: [[]],
      };

      for (let i = 0; i < results.documents[0].length; i++) {
        const filePath = results.metadatas[0][i].file_path;
        if (!filePath.includes(reference_path)) {
          filtered.documents[0].push(results.documents[0][i]);
          filtered.metadatas[0].push(results.metadatas[0][i]);
          filtered.distances[0].push(results.distances[0][i]);
        }
      }

      if (filtered.documents[0].length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `No similar code found for: ${reference_path}`,
            },
          ],
        };
      }

      const formattedResults = filtered.documents[0]
        .slice(0, n_results)
        .map((doc, idx) => {
          const metadata = filtered.metadatas[0][idx];
          const distance = filtered.distances[0][idx];
          const similarity = (1 - distance).toFixed(3);

          return `## Similar Code ${idx + 1} (similarity: ${similarity})
**File**: ${metadata.file_path}:${metadata.start_line || "1"}

\`\`\`${metadata.language || ""}
${doc}
\`\`\`
`;
        })
        .join("\n\n---\n\n");

      return {
        content: [
          {
            type: "text",
            text: `# 🔗 Code Similar to: "${reference_path}"

**Found**: ${filtered.documents[0].slice(0, n_results).length} similar implementations

---

${formattedResults}`,
          },
        ],
      };
    } catch (error) {
      throw error;
    }
  }

  // ============================================================================
  // TRACE EXECUTION PATHS
  // ============================================================================

  async tracePath(args) {
    const { start_point, end_point, include_depth = 15 } = args;

    try {
      const collection = await this.chromaClient.getCollection({
        name: COLLECTION_NAME,
        embeddingFunction: this.embeddingFunction,
      });

      // Build query to find execution path
      const pathQuery = end_point
        ? `${start_point} navigation flow execution path to ${end_point} routing navigation calls function calls`
        : `${start_point} navigation flow execution path routing navigation screen transition`;

      const where = this.buildWhereClause();
      const results = await collection.query({
        queryTexts: [pathQuery],
        nResults: include_depth,
        where: where,
      });

      if (!results.documents[0] || results.documents[0].length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `No execution path found from: "${start_point}"${end_point ? ` to: "${end_point}"` : ''}`,
            },
          ],
        };
      }

      const formattedResults = results.documents[0]
        .map((doc, idx) => {
          const metadata = results.metadatas[0][idx];
          const distance = results.distances[0][idx];
          const similarity = (1 - distance).toFixed(3);

          return `## Step ${idx + 1} (relevance: ${similarity})
**File**: ${metadata.file_path}:${metadata.start_line || "1"}

\`\`\`${metadata.language || ""}
${doc}
\`\`\`
`;
        })
        .join("\n\n---\n\n");

      return {
        content: [
          {
            type: "text",
            text: `# 🛤️ Execution Path Trace

**Start**: ${start_point}
${end_point ? `**End**: ${end_point}` : ''}
**Depth**: ${results.documents[0].length} code segments analyzed

**🤖 TO THE AI**: The code below shows the execution path. Analyze these segments and:
1. Identify the sequence of function/component calls
2. Explain how the flow moves from start ${end_point ? `to end` : `through the system`}
3. Highlight key decision points and navigation logic
4. Provide step-by-step instructions if needed

---

${formattedResults}`,
          },
        ],
      };
    } catch (error) {
      throw error;
    }
  }

  // ============================================================================
  // FIND REPRODUCTION STEPS
  // ============================================================================

  async findReproduction(args) {
    const { target, context } = args;

    try {
      const collection = await this.chromaClient.getCollection({
        name: COLLECTION_NAME,
        embeddingFunction: this.embeddingFunction,
      });

      // Build query for reproduction steps
      const reproQuery = `${target} entry point trigger reproduce how to reach navigation steps ${context || ''}`;

      const where = this.buildWhereClause();
      const results = await collection.query({
        queryTexts: [reproQuery],
        nResults: 12,
        where: where,
      });

      if (!results.documents[0] || results.documents[0].length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `Could not find reproduction steps for: "${target}"`,
            },
          ],
        };
      }

      const formattedResults = results.documents[0]
        .map((doc, idx) => {
          const metadata = results.metadatas[0][idx];
          const distance = results.distances[0][idx];
          const similarity = (1 - distance).toFixed(3);

          return `## Code Segment ${idx + 1} (relevance: ${similarity})
**File**: ${metadata.file_path}:${metadata.start_line || "1"}

\`\`\`${metadata.language || ""}
${doc}
\`\`\`
`;
        })
        .join("\n\n---\n\n");

      return {
        content: [
          {
            type: "text",
            text: `# 🐛 Reproduction Steps for: "${target}"

${context ? `**Context**: ${context}` : ''}
**Relevant Code**: ${results.documents[0].length} segments found

**🤖 TO THE AI**: Using the code below, provide:
1. Step-by-step instructions to reproduce/trigger the target
2. Entry points (screens, buttons, navigation paths)
3. Prerequisites or setup needed
4. Expected behavior at each step

---

${formattedResults}`,
          },
        ],
      };
    } catch (error) {
      throw error;
    }
  }

  // ============================================================================
  // MAP DEPENDENCIES
  // ============================================================================

  async mapDependencies(args) {
    const { file_path, direction = "both", depth = 2 } = args;

    try {
      const collection = await this.chromaClient.getCollection({
        name: COLLECTION_NAME,
        embeddingFunction: this.embeddingFunction,
      });

      // Get the target file
      const where = this.buildWhereClause({ file_path: { $contains: file_path } });
      const targetFile = await collection.get({
        where: where,
        limit: 1,
        include: ["documents", "metadatas"],
      });

      if (!targetFile.documents || targetFile.documents.length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `File not found: ${file_path}`,
            },
          ],
        };
      }

      // Build queries based on direction
      const queries = [];
      if (direction === "imports" || direction === "both") {
        queries.push(`import require dependency from ${targetFile.documents[0].substring(0, 500)}`);
      }
      if (direction === "imported_by" || direction === "both") {
        queries.push(`exports function class component from ${file_path}`);
      }

      const allResults = [];
      const depWhere = this.buildWhereClause();
      for (const query of queries) {
        const results = await collection.query({
          queryTexts: [query],
          nResults: 10 * depth,
          where: depWhere,
        });
        allResults.push(...results.documents[0].map((doc, idx) => ({
          doc,
          metadata: results.metadatas[0][idx],
          distance: results.distances[0][idx],
        })));
      }

      // Deduplicate and filter out self
      const seen = new Set();
      const filtered = allResults.filter(item => {
        const key = item.metadata.file_path;
        if (key.includes(file_path) || seen.has(key)) return false;
        seen.add(key);
        return true;
      });

      if (filtered.length === 0) {
        return {
          content: [
            {
              type: "text",
              text: `No dependencies found for: ${file_path}`,
            },
          ],
        };
      }

      const formattedResults = filtered
        .slice(0, 15)
        .map((item, idx) => {
          const similarity = (1 - item.distance).toFixed(3);
          return `## Dependency ${idx + 1} (relevance: ${similarity})
**File**: ${item.metadata.file_path}:${item.metadata.start_line || "1"}

\`\`\`${item.metadata.language || ""}
${item.doc}
\`\`\`
`;
        })
        .join("\n\n---\n\n");

      return {
        content: [
          {
            type: "text",
            text: `# 📊 Dependency Map for: "${file_path}"

**Direction**: ${direction}
**Depth**: ${depth}
**Found**: ${filtered.slice(0, 15).length} related files

**🤖 TO THE AI**: The code below shows dependencies. Analyze and identify:
1. What this file imports/requires
2. What files import/use this file
3. The dependency relationships and data flow
4. Create a dependency tree/graph if helpful

---

${formattedResults}`,
          },
        ],
      };
    } catch (error) {
      throw error;
    }
  }

  // ============================================================================
  // STATISTICS
  // ============================================================================

  async getStats() {
    try {
      const collection = await this.chromaClient.getCollection({
        name: COLLECTION_NAME,
        embeddingFunction: this.embeddingFunction,
      });

      const count = await collection.count();
      const freshness = await this.checkIndexFreshness(collection);

      let freshnessWarning = "";
      if (freshness && freshness.stale) {
        freshnessWarning = `\n\n⚠️  **Index is stale**\nIndexed: ${freshness.indexed}\nCurrent: ${freshness.current}\n\nRun: docker compose up -d`;
      }

      return {
        content: [
          {
            type: "text",
            text: `# 📊 Vector Store Statistics

**Collection**: ${COLLECTION_NAME}
**Documents**: ${count}
**ChromaDB**: ${CHROMA_HOST}:${CHROMA_PORT}${freshnessWarning}

Ready for queries.`,
          },
        ],
      };
    } catch (error) {
      if (error.message.includes("does not exist")) {
        return {
          content: [
            {
              type: "text",
              text: `Collection "${COLLECTION_NAME}" does not exist. Run: docker compose up -d`,
            },
          ],
        };
      }
      throw error;
    }
  }

  // ============================================================================
  // HELPER: CHECK INDEX FRESHNESS
  // ============================================================================

  async checkIndexFreshness(collection) {
    try {
      const { execSync } = await import('child_process');
      const codebasePath = process.env.CODEBASE_PATH;

      if (!codebasePath) return null;

      let currentGitHash = "";
      let currentBranch = "";
      try {
        currentGitHash = execSync('git rev-parse HEAD', {
          cwd: codebasePath,
          encoding: 'utf-8'
        }).trim();
        currentBranch = execSync('git rev-parse --abbrev-ref HEAD', {
          cwd: codebasePath,
          encoding: 'utf-8'
        }).trim();
      } catch (e) {
        return null;
      }

      const where = this.buildWhereClause();
      const sample = await collection.get({
        where: where,
        limit: 1,
        include: ["metadatas"],
      });

      if (sample.metadatas && sample.metadatas.length > 0) {
        const indexedHash = sample.metadatas[0].git_commit || sample.metadatas[0].git_hash;
        const indexedBranch = sample.metadatas[0].git_branch;

        // Check if branch changed
        if (indexedBranch && currentBranch && indexedBranch !== currentBranch) {
          return {
            stale: true,
            indexed: `${indexedBranch}@${indexedHash?.substring(0, 8) || 'unknown'}`,
            current: `${currentBranch}@${currentGitHash.substring(0, 8)}`,
            reason: 'branch_changed'
          };
        }

        // Check if commit changed
        if (indexedHash && currentGitHash && indexedHash !== currentGitHash) {
          return {
            stale: true,
            indexed: indexedHash.substring(0, 8),
            current: currentGitHash.substring(0, 8),
            reason: 'commit_changed'
          };
        }
      }

      return { stale: false };
    } catch (error) {
      console.error("Error checking freshness:", error);
      return null;
    }
  }

  // ============================================================================
  // SERVER RUNNER
  // ============================================================================

  async run() {
    const transport = new StdioServerTransport();
    await this.server.connect(transport);
    console.error("Vector MCP Server v2.0 running on stdio");
  }
}

const server = new VectorMCPServer();
server.run().catch(console.error);
