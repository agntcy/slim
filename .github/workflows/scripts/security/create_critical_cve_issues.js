const { Octokit } = require('@octokit/rest');

// Initialize Octokit with token
const octokit = new Octokit({
  auth: process.env.GITHUB_TOKEN
});

// Configuration
const config = {
  owner: process.env.GITHUB_REPOSITORY.split('/')[0],
  repo: process.env.GITHUB_REPOSITORY.split('/')[1],
  labels: ['security', 'critical-cve', 'trivy-scan'],
  assignees: [],
  priorityThreshold: 'CRITICAL' // Only create issues for CRITICAL vulnerabilities
};

async function main() {
  try {
    // Read Trivy scan results
    const fs = require('fs');
    const path = 'trivy-results.json';

    if (!fs.existsSync(path)) {
      console.log('No Trivy results file found');
      return;
    }

    const trivyResults = JSON.parse(fs.readFileSync(path, 'utf8'));

    // Process scan results
    for (const result of trivyResults.Results || []) {
      if (!result.Vulnerabilities) continue;

      // Filter critical vulnerabilities
      const criticalVulns = result.Vulnerabilities.filter(
        vuln => vuln.Severity === config.priorityThreshold
      );

      for (const vuln of criticalVulns) {
        await createOrUpdateIssue(vuln, result.Target);
      }
    }

    console.log('Finished processing vulnerabilities');

  } catch (error) {
    console.error('Error:', error);
    process.exit(1);
  }
}

async function createOrUpdateIssue(vulnerability, target) {
  const issueTitle = `🚨 Critical CVE: ${vulnerability.VulnerabilityID} in ${target}`;
  const issueBody = generateIssueBody(vulnerability, target);

  try {
    // Check if issue already exists
    const existingIssues = await octokit.rest.issues.listForRepo({
      owner: config.owner,
      repo: config.repo,
      state: 'open',
      labels: config.labels.join(',')
    });

    const existingIssue = existingIssues.data.find(
      issue => issue.title.includes(vulnerability.VulnerabilityID)
    );

    if (existingIssue) {
      console.log(`Issue already exists for ${vulnerability.VulnerabilityID}`);
      return;
    }

    // Create new issue
    const newIssue = await octokit.rest.issues.create({
      owner: config.owner,
      repo: config.repo,
      title: issueTitle,
      body: issueBody,
      labels: config.labels,
      assignees: config.assignees
    });

    console.log(`Created issue #${newIssue.data.number} for ${vulnerability.VulnerabilityID}`);

  } catch (error) {
    console.error(`Failed to create issue for ${vulnerability.VulnerabilityID}:`, error);
  }
}

function generateIssueBody(vulnerability, target) {
  return `## 🚨 Critical Security Vulnerability Detected

**CVE ID:** ${vulnerability.VulnerabilityID}
**Severity:** ${vulnerability.Severity}
**Target:** ${target}
**Package:** ${vulnerability.PkgName} ${vulnerability.InstalledVersion}

### Description
${vulnerability.Description || 'No description available'}

### CVSS Score
${vulnerability.CVSS ? `**Score:** ${vulnerability.CVSS.nvd?.V3Score || vulnerability.CVSS.redhat?.V3Score || 'N/A'}` : 'No CVSS data available'}

### References
${vulnerability.References ? vulnerability.References.map(ref => `- ${ref}`).join('\n') : 'No references available'}

### Recommended Action
${vulnerability.FixedVersion ? `Update to version **${vulnerability.FixedVersion}** or later` : 'No fix currently available - monitor for updates'}

### Additional Information
- **Vulnerability DB:** ${vulnerability.DataSource?.Name || 'Unknown'}
- **Published:** ${vulnerability.PublishedDate || 'Unknown'}
- **Last Modified:** ${vulnerability.LastModifiedDate || 'Unknown'}

---
*This issue was automatically created by our security scanning workflow. Please prioritize resolution due to the critical severity level.*`;
}

main().catch(console.error);
