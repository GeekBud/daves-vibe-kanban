import { useRef, forwardRef, useCallback } from 'react';
import { cn } from '../lib/cn';
import { useSkillAutocomplete, type Skill } from '../hooks/useSkills';
import { SkillsAutocomplete } from './SkillsAutocomplete';
import { LightningIcon } from '@phosphor-icons/react';

interface SkillsInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit?: () => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  workspaceId?: string;
}

export const SkillsInput = forwardRef<HTMLInputElement, SkillsInputProps>(
  function SkillsInput(
    { value, onChange, onSubmit, placeholder, disabled, className, workspaceId },
    ref
  ) {
    const {
      showDropdown,
      filteredSkills,
      selectedIndex,
      insertSkill,
      handleKeyDown: handleAutocompleteKeyDown,
      closeDropdown,
    } = useSkillAutocomplete(value, { workspaceId });

    const containerRef = useRef<HTMLDivElement>(null);
    const inputContainerRef = useRef<HTMLDivElement>(null);

    const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange(e.target.value);
    };

    // Helper to insert skill and update value
    const handleInsertSkill = useCallback((skill: Skill) => {
      const newValue = insertSkill(skill);
      onChange(newValue);
    }, [insertSkill, onChange]);

    const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
      // Handle autocomplete navigation
      const result = handleAutocompleteKeyDown(e);
      
      // If result is an object with selected skill, update the value
      if (result && typeof result === 'object' && result.selected && result.value) {
        onChange(result.value);
        return;
      }
      
      // If handled by autocomplete, stop here
      if (result) return;

      // Handle submit on Enter
      if (e.key === 'Enter' && !e.shiftKey && onSubmit) {
        e.preventDefault();
        onSubmit();
      }
    };

    const handleSelectSkill = (skill: Skill) => {
      handleInsertSkill(skill);
    };

    return (
      <div ref={containerRef} className={cn('relative', className)}>
        <div ref={inputContainerRef} className="relative">
          <LightningIcon className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-low" />
          <input
            ref={ref}
            type="text"
            value={value}
            onChange={handleChange}
            onKeyDown={handleKeyDown}
            placeholder={placeholder}
            disabled={disabled}
            className={cn(
              'w-full pl-9 pr-3 py-2 text-sm rounded-md border border-input',
              'bg-background text-foreground',
              'placeholder:text-low',
              'focus:outline-none focus:ring-2 focus:ring-ring focus:border-input',
              'disabled:opacity-50 disabled:cursor-not-allowed'
            )}
          />
        </div>
        <SkillsAutocomplete
          skills={filteredSkills}
          selectedIndex={selectedIndex}
          onSelect={handleSelectSkill}
          onClose={closeDropdown}
          visible={showDropdown}
          anchorRef={inputContainerRef}
        />
      </div>
    );
  }
);
