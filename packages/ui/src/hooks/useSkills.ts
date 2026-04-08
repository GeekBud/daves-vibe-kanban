import { useState, useEffect, useCallback, useMemo } from 'react';

export interface Skill {
  id: string;
  name: string;
  description: string;
  source?: 'built-in' | 'local';
}

// Mock skills data - in production this would come from an API
const MOCK_SKILLS: Skill[] = [
  { id: 'ask', name: 'Ask', description: 'Ask the codebase questions', source: 'built-in' },
  { id: 'git-ai-search', name: 'Git AI Search', description: 'Search git history with AI', source: 'built-in' },
  { id: 'prompt-analysis', name: 'Prompt Analysis', description: 'Analyze prompt patterns', source: 'built-in' },
  { id: 'daves-vibe-guide', name: 'Daves Vibe Guide', description: 'Fork-Track system guide', source: 'built-in' },
  { id: 'daves-vibe-check', name: 'Daves Vibe Check', description: 'Check upstream updates', source: 'built-in' },
  { id: 'daves-vibe-sync', name: 'Daves Vibe Sync', description: 'Sync with upstream', source: 'built-in' },
  { id: 'daves-vibe-review', name: 'Daves Vibe Review', description: 'Review upstream commits', source: 'built-in' },
  { id: 'daves-vibe-mod', name: 'Daves Vibe Mod', description: 'Record mod decisions', source: 'built-in' },
];

export interface UseSkillsOptions {
  workspaceId?: string;
}

export function useSkills(options: UseSkillsOptions = {}) {
  const { workspaceId } = options;
  const [skills, setSkills] = useState<Skill[]>(MOCK_SKILLS);
  const [loading, setLoading] = useState(false);

  // Load skills from backend (built-in + local .cursor/skills/)
  useEffect(() => {
    const loadSkills = async () => {
      setLoading(true);
      try {
        // Build URL with optional workspaceId
        const url = workspaceId 
          ? `/api/skills?workspace_id=${workspaceId}`
          : '/api/skills';
        
        console.log('[useSkills] Loading skills from:', url);
        
        const response = await fetch(url);
        if (response.ok) {
          const data = await response.json() as Skill[];
          console.log('[useSkills] Loaded skills:', data.length, data);
          setSkills(data);
        } else {
          console.error('[useSkills] API error:', response.status, response.statusText);
          // Fallback to mock skills if API fails
          setSkills(MOCK_SKILLS);
        }
      } catch (error) {
        console.error('[useSkills] Error loading skills:', error);
        // Fallback to mock skills on error
        setSkills(MOCK_SKILLS);
      } finally {
        setLoading(false);
      }
    };

    loadSkills();
  }, [workspaceId]);

  const searchSkills = useCallback((query: string): Skill[] => {
    if (!query.trim()) return skills;
    const lowerQuery = query.toLowerCase();
    return skills.filter(
      (skill) =>
        skill.name.toLowerCase().includes(lowerQuery) ||
        skill.description.toLowerCase().includes(lowerQuery) ||
        skill.id.toLowerCase().includes(lowerQuery)
    );
  }, [skills]);

  return {
    skills,
    loading,
    searchSkills,
  };
}

export interface UseSkillAutocompleteOptions {
  workspaceId?: string;
  onSkillSelect?: (skill: Skill) => void;
}

export function useSkillAutocomplete(
  inputValue: string, 
  options: UseSkillAutocompleteOptions = {}
) {
  const { workspaceId, onSkillSelect } = options;
  const [showDropdown, setShowDropdown] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const { searchSkills } = useSkills({ workspaceId });

  // Check if input contains @ trigger
  const { searchQuery, cursorPosition, isTriggered } = useMemo(() => {
    const lastAtIndex = inputValue.lastIndexOf('@');
    
    if (lastAtIndex === -1) {
      return { searchQuery: '', cursorPosition: -1, isTriggered: false };
    }
    
    // Check if @ is at word boundary (start of string or after space)
    const charBeforeAt = inputValue[lastAtIndex - 1];
    if (lastAtIndex > 0 && charBeforeAt !== ' ' && charBeforeAt !== '\n') {
      return { searchQuery: '', cursorPosition: -1, isTriggered: false };
    }
    
    const query = inputValue.slice(lastAtIndex + 1);
    // Don't trigger if there's a space after @
    if (query.includes(' ') || query.includes('\n')) {
      return { searchQuery: '', cursorPosition: -1, isTriggered: false };
    }
    
    return { 
      searchQuery: query, 
      cursorPosition: lastAtIndex,
      isTriggered: true 
    };
  }, [inputValue]);

  const filteredSkills = useMemo(() => {
    if (!isTriggered) return [];
    return searchSkills(searchQuery);
  }, [isTriggered, searchQuery, searchSkills]);

  // Reset selected index when filtered skills change
  useEffect(() => {
    setSelectedIndex(0);
    setShowDropdown(filteredSkills.length > 0 && isTriggered);
  }, [filteredSkills.length, isTriggered]);

  const insertSkill = useCallback((skill: Skill) => {
    if (cursorPosition === -1) return inputValue;
    
    const beforeAt = inputValue.slice(0, cursorPosition);
    const afterQuery = inputValue.slice(cursorPosition + 1 + searchQuery.length);
    const newValue = `${beforeAt}@${skill.name} ${afterQuery}`;
    
    setShowDropdown(false);
    onSkillSelect?.(skill);
    return newValue;
  }, [inputValue, cursorPosition, searchQuery, onSkillSelect]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (!showDropdown) return false;

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setSelectedIndex((prev) => 
          prev < filteredSkills.length - 1 ? prev + 1 : prev
        );
        return true;
      case 'ArrowUp':
        e.preventDefault();
        setSelectedIndex((prev) => (prev > 0 ? prev - 1 : 0));
        return true;
      case 'Enter':
      case 'Tab':
        e.preventDefault();
        if (filteredSkills[selectedIndex]) {
          // Return the new value so parent can update
          return { selected: true, value: insertSkill(filteredSkills[selectedIndex]) };
        }
        return { selected: false };
      case 'Escape':
        setShowDropdown(false);
        return { selected: false, close: true };
      default:
        return false;
    }
  }, [showDropdown, filteredSkills, selectedIndex, insertSkill]);

  return {
    showDropdown,
    filteredSkills,
    selectedIndex,
    insertSkill,
    handleKeyDown,
    isTriggered,
    closeDropdown: () => setShowDropdown(false),
  };
}
