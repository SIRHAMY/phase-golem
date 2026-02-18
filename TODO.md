
A list of todos we should get to, roughly ordered by priority.

# P0

* Add a research / roadmap assistant - for project roadmaps, ideas, product ideas, new features, etc
- Change changes reviews to at end propose suggested things and then have an interview phase where we go over them in small batches so I can ask questions and give decisions for what we should do.
- Add ability to configure where the phase golem config is located 

# P1

* Figure out how to do localhost runs from within devcontainer - my port forwarding doesn't quite seem to work
* Add to style guide - try and use specific data classes when possible with named params vs using primitives to hold state
* Consider rename the orchestrator to something more like gardener or oracle or tender or smth?
* Consider design subagents for ui/ux, technical architecture - deep dive into each of these?
* Align orchestrator on 1 config format - yaml or toml
- Orchestrator should have a way to point to a backlog file. That way there could be a global backlog file or ppl could run their own version on their own backlogs? Maybe also point to different pipeline config file? That could be a cool way to allow ppl to run it in repos they don't fully control.
- Orchestrator phases should have an ability to update the ratings of a work item. Like if triage thinks its small and then after tech research or design or speccing we think its large and risky then we should have a method for backfilling that data
- Double check that we're moving work through - it seems like we're going wide vs deep?
- Clean up the code to be more functional. Read, transform, save.
- Would be nice to have a thing that does checking if an item is actually worth doing after each phase? Like is this still worth the impact we had? And move it to blocked if not? Basically if the imapct is ever less than a cost thing
- Might be good to have a way to easily go through the tasks we have available / up next and work on them manually - should be an easy way to do it manually just like using claude code - but maybe at that point you just do it yourself and use similar artifacts so the golem can read it? 
- Consider optimize the agents used for certain tasks - like use a cheap agent to do this thing fast, use an expensive agent to do this thing
- Better way to handle duplicate / not needed items in backlog
- Allow using different models via the orchestrator
- Allow configuring different models / different runners (opencode, codex, claude) for various phases and as default. Useful for cost management or model tiering.

# P2

- Use LightClone and AsyncGuard in orchestrator grovegolem 
- The orchestrator's numbering of work items doesn't seem right. We need to have it auto increment reliably. Might mean holding onto old phases and what they were in an archive somewhere so we don't reuse them? 
- When we say scheduling for a work item - we should include its human readable title, so that it's more clear what that work item is at a glance